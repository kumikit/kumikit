//! `kumikit update`: check for and install a newer signed `kumikit` core release
//! (`docs/spec/10-distribution.md#application-updates`,
//! [ADR 0031](../../../docs/decisions/0031-signed-release-manifest-and-updates.md)).

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use kumikit_protocol::APP_VERSION;
use kumikit_protocol::methods::{CoreActivity, CoreShutdown, Empty, ShutdownParams};
use kumikit_registry::release::{ReleaseClient, current_target, extract_core_binary};

use super::{Out, Rpc, Target, confirm};

/// Options for `kumikit update`.
#[derive(Debug, Clone, Copy, Default)]
pub struct UpdateOptions {
    /// Only check for updates; do not download or install.
    pub check: bool,
    /// Proceed without interactive confirmation.
    pub yes: bool,
    /// Poll `core.activity` until all agents are idle before shutting down a running core.
    pub wait_idle: bool,
    /// Stop a running core even if agents are active.
    pub force: bool,
}

/// Checks for and installs a newer signed `kumikit` binary for the current platform.
pub async fn self_update(target: &Target, options: UpdateOptions, out: &mut dyn Out) -> Result<()> {
    let client = ReleaseClient::from_env(&crate::paths::process_env)
        .context("cannot configure release client")?;
    let exe = std::env::current_exe().context("cannot determine current kumikit binary path")?;
    self_update_with(
        &client,
        APP_VERSION,
        current_target(),
        &exe,
        target,
        options,
        out,
    )
    .await
}

/// Testable core of [`self_update`].
pub async fn self_update_with(
    client: &ReleaseClient,
    current_version: &str,
    target_triple: &str,
    target_exe: &Path,
    core_target: &Target,
    options: UpdateOptions,
    out: &mut dyn Out,
) -> Result<()> {
    let check = client
        .check_update(current_version)
        .await
        .context("update check failed")?;
    if !check.update_available {
        out.line(&format!(
            "Kumikit {} is up to date (latest signed release: {}).",
            check.current_version, check.latest_version
        ));
        return Ok(());
    }
    let Some(asset) = check.manifest.core.get(target_triple) else {
        bail!(
            "release {} does not include a binary for {target_triple}",
            check.latest_version
        );
    };
    out.line(&format!(
        "Update available: {} -> {} ({})",
        check.current_version, check.latest_version, check.manifest.notes_url
    ));
    if options.check {
        return Ok(());
    }
    if !options.yes
        && !confirm(&format!(
            "Download and install Kumikit {}?",
            check.latest_version
        ))
    {
        out.line("Update cancelled.");
        return Ok(());
    }

    let stage = tempfile::tempdir().context("cannot create staging directory")?;
    let archive_name = if asset.format == "zip" {
        "kumikit.zip"
    } else {
        "kumikit.tar.gz"
    };
    let archive_path = stage.path().join(archive_name);
    out.line(&format!("Downloading and verifying {}...", asset.url));
    client
        .download_verified(asset, &archive_path)
        .await
        .context("failed to download or verify release archive")?;
    let new_bin = extract_core_binary(&archive_path, &asset.format, &stage.path().join("bin"))
        .context("failed to extract kumikit binary")?;

    let was_running = stop_running_core(core_target, options, out).await?;
    replace_binary(&new_bin, target_exe)
        .with_context(|| format!("failed to replace {}", target_exe.display()))?;
    out.line(&format!(
        "Installed Kumikit {} to {}.",
        check.latest_version,
        target_exe.display()
    ));
    if was_running {
        out.line("Stopped the previous running core. Start it again with `kumikit serve` (or reconnect from the desktop app).");
    }
    Ok(())
}

/// Checks if a local core is running and shuts it down cleanly, respecting idle guards.
async fn stop_running_core(
    target: &Target,
    options: UpdateOptions,
    out: &mut dyn Out,
) -> Result<bool> {
    let mut rpc = match Rpc::connect(&target.address).await {
        Ok(r) => r,
        Err(_) => return Ok(false),
    };
    let mut announced_wait = false;
    loop {
        let activity = rpc
            .call::<CoreActivity>(&Empty {})
            .await
            .context("cannot query running core activity")?;
        if activity.idle || options.force {
            match rpc
                .call::<CoreShutdown>(&ShutdownParams {
                    if_idle: (!options.force).then_some(true),
                })
                .await
            {
                Ok(_) => break,
                Err(e)
                    if e.data.code == kumikit_protocol::ErrorCode::Conflict
                        && options.wait_idle =>
                {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
                Err(e) => bail!("cannot stop running core: {}", e.message),
            }
        }
        if !options.wait_idle {
            bail!(
                "refusing to interrupt {} active turn(s) and {} queued message(s); re-run with --wait-idle or --force",
                activity.active_turns,
                activity.ready_queued
            );
        }
        if !announced_wait {
            out.line(&format!(
                "Waiting for {} active turn(s) and {} queued message(s) to finish...",
                activity.active_turns, activity.ready_queued
            ));
            announced_wait = true;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    drop(rpc);
    // Give the core a moment to exit and release its socket/file handles.
    for _ in 0..40 {
        if crate::transport::connect(&target.address).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(true)
}

/// Atomically replaces `target_exe` with `new_bin` (creating the temp file in the same
/// directory so `rename` stays on one filesystem).
pub fn replace_binary(new_bin: &Path, target_exe: &Path) -> std::io::Result<()> {
    let parent = target_exe
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    let mut src = std::fs::File::open(new_bin)?;
    std::io::copy(&mut src, tmp.as_file_mut())?;
    tmp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755))?;
        tmp.persist(target_exe).map_err(|e| e.error)?;
    }
    #[cfg(windows)]
    {
        // Windows cannot overwrite a running executable directly, but it can be renamed aside.
        let backup = target_exe.with_extension("old.exe");
        let _ = std::fs::remove_file(&backup);
        if target_exe.exists() {
            std::fs::rename(target_exe, &backup)?;
        }
        if let Err(e) = tmp.persist(target_exe) {
            let _ = std::fs::rename(&backup, target_exe);
            return Err(e.error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use kumikit_package::signature::SigningKey;
    use kumikit_registry::fetch::Fetcher;
    use kumikit_registry::release::{
        RELEASE_MANIFEST_FORMAT, ReleaseAsset, ReleaseManifest, sha256_bytes_hex, sign_manifest,
    };

    use super::*;

    struct Capture(Vec<String>);
    impl Out for Capture {
        fn line(&mut self, text: &str) {
            self.0.push(text.to_owned());
        }
    }

    fn make_archive(dir: &Path, payload: &[u8]) -> std::path::PathBuf {
        let archive = dir.join("kumikit.tar.gz");
        let f = std::fs::File::create(&archive).unwrap();
        let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        let name = if cfg!(windows) {
            "kumikit.exe"
        } else {
            "kumikit"
        };
        tar.append_data(&mut header, name, payload).unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        archive
    }

    fn file_url(path: &Path) -> String {
        let s = path.to_string_lossy().replace('\\', "/");
        if s.starts_with('/') {
            format!("file://{s}")
        } else {
            format!("file:///{s}")
        }
    }

    #[tokio::test]
    async fn self_update_verifies_and_replaces_binary() {
        let dir = tempfile::tempdir().unwrap();
        let (key, _) = SigningKey::generate().unwrap();
        let payload = b"#!/bin/sh\necho updated-binary\n";
        let archive = make_archive(dir.path(), payload);
        let archive_bytes = std::fs::read(&archive).unwrap();

        let mut core = BTreeMap::new();
        core.insert(
            current_target().to_owned(),
            ReleaseAsset {
                url: file_url(&archive),
                sha256: sha256_bytes_hex(&archive_bytes),
                size: archive_bytes.len() as u64,
                format: "tar.gz".into(),
            },
        );
        let manifest = ReleaseManifest {
            format: RELEASE_MANIFEST_FORMAT,
            version: "0.2.0".into(),
            tag: "v0.2.0".into(),
            published_at: "2026-11-25T00:00:00Z".into(),
            notes_url: "https://github.com/kumikit/kumikit/releases/tag/v0.2.0".into(),
            notes: None,
            core,
            desktop: BTreeMap::new(),
        };
        let (json, sig) = sign_manifest(&manifest, &key).unwrap();
        let manifest_path = dir.path().join("release.json");
        std::fs::write(&manifest_path, &json).unwrap();
        std::fs::write(dir.path().join("release.json.sig"), &sig).unwrap();

        let client =
            ReleaseClient::with_config(Fetcher::default(), file_url(&manifest_path), key.public());

        let target_exe = dir.path().join("kumikit");
        std::fs::write(&target_exe, b"old-binary").unwrap();
        let core_target = Target {
            data_dir: dir.path().join("data"),
            address: dir
                .path()
                .join("no-core.sock")
                .to_string_lossy()
                .into_owned(),
        };

        // Check-only leaves binary untouched.
        let mut out = Capture(Vec::new());
        self_update_with(
            &client,
            "0.1.0",
            current_target(),
            &target_exe,
            &core_target,
            UpdateOptions {
                check: true,
                yes: true,
                wait_idle: false,
                force: false,
            },
            &mut out,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&target_exe).unwrap(), b"old-binary");
        assert!(
            out.0
                .iter()
                .any(|l| l.contains("Update available: 0.1.0 -> 0.2.0"))
        );

        // Full update replaces binary atomically.
        let mut out = Capture(Vec::new());
        self_update_with(
            &client,
            "0.1.0",
            current_target(),
            &target_exe,
            &core_target,
            UpdateOptions {
                check: false,
                yes: true,
                wait_idle: false,
                force: false,
            },
            &mut out,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&target_exe).unwrap(), payload);
    }
}
