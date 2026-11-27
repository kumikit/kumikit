//! Matching remote `kumikit` installation and version sync over SSH
//! (`docs/spec/06-client.md#shell-rust-kumikit-shell`,
//! [ADR 0032](../../../docs/decisions/0032-ssh-remote-core-sync.md)).
//!
//! Downloads and Ed25519/SHA-256-verifies the matching `kumikit-<tag>-<target>.tar.gz`
//! archive **locally on the client**, checks remote `core.activity` before stopping an
//! older running core, and streams the archive over `ssh` `stdin` to
//! `$HOME/.local/bin/kumikit`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use kumikit_protocol::{APP_VERSION, PROTOCOL_VERSION};
use kumikit_registry::release::{ReleaseClient, normalize_version, target_for_uname};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::connections::CommandError;
use crate::profiles::{Kind, Profiles};
use crate::ssh;

/// Remote command for a user-installed binary in `$HOME/.local/bin`.
pub const LOCAL_BIN_BRIDGE_COMMAND: &str = "$HOME/.local/bin/kumikit bridge";

/// Target of an `ssh_probe` or `ssh_sync` call: either a saved SSH profile name (so the
/// webview never learns its destination) or a new destination being added.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTargetRequest {
    /// Saved SSH profile name.
    #[serde(default)]
    pub profile: Option<String>,
    /// New SSH destination (`user@host` or `~/.ssh/config` host), when not yet saved.
    #[serde(default)]
    pub destination: Option<String>,
    /// Optional custom remote bridge command.
    #[serde(default)]
    pub command: Option<String>,
    /// When true (default), refuse with `busy` if any remote agent turn or ready queued
    /// message is active before restarting an older running remote core.
    #[serde(default = "default_true")]
    pub if_idle: bool,
}

fn default_true() -> bool {
    true
}

/// Result of `ssh_probe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshProbeResult {
    /// Remote `uname -s` (for example `Linux` or `Darwin`).
    pub os: String,
    /// Remote `uname -m` (for example `x86_64` or `aarch64`).
    pub arch: String,
    /// Matched Rust release target triple, if supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Whether official Kumikit core archives exist for this remote platform.
    pub supported: bool,
    /// Path to `kumikit` found on the remote host, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_binary: Option<String>,
    /// Reported `kumikit --version` on the remote host, if installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_version: Option<String>,
    /// This desktop app's version.
    pub local_version: String,
    /// `"match"`, `"missing"`, `"mismatch"`, or `"unsupported"`.
    pub status: &'static str,
}

/// Result of `ssh_sync`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshSyncResult {
    /// Installed version on the remote host.
    pub installed_version: String,
    /// Remote binary path (`$HOME/.local/bin/kumikit` or expanded path).
    pub remote_binary: String,
    /// True when the sync started a detached remote `kumikit serve`.
    pub started_core: bool,
    /// Updated profile command if the saved profile was pointed to `$HOME/.local/bin/kumikit bridge`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_command: Option<String>,
}

// All remote helper scripts are wrapped in `sh -c '...'` with no embedded single quotes
// and no file globs so they run identically regardless of the remote user's login shell
// (`bash`, `zsh` with `NOMATCH`, `fish`, `csh`/`tcsh`, or `dash`).
const PROBE_SCRIPT: &str = concat!(
    "sh -c '",
    "printf \"OS=%s\\nARCH=%s\\n\" \"$(uname -s)\" \"$(uname -m)\"; ",
    "BIN=$(command -v kumikit 2>/dev/null || true); ",
    "if [ -z \"$BIN\" ] && [ -x \"$HOME/.local/bin/kumikit\" ]; then BIN=\"$HOME/.local/bin/kumikit\"; fi; ",
    "printf \"BIN=%s\\n\" \"$BIN\"; ",
    "if [ -n \"$BIN\" ]; then VER=$(\"$BIN\" --version 2>/dev/null | head -n 1 || true); else VER=\"\"; fi; ",
    "printf \"VER=%s\\n\" \"$VER\"",
    "'"
);

const INSTALL_STREAM_SCRIPT: &str = concat!(
    "sh -c '",
    "set -eu; ",
    "mkdir -p \"$HOME/.local/bin\"; ",
    "tmp=$(mktemp -d \"$HOME/.local/bin/.kumikit-install.XXXXXX\"); ",
    "cleanup() { rm -rf \"$tmp\"; }; ",
    "trap cleanup EXIT; ",
    "tar -xzf - -C \"$tmp\"; ",
    "chmod 755 \"$tmp/kumikit\"; ",
    "mv -f \"$tmp/kumikit\" \"$HOME/.local/bin/kumikit\"; ",
    "if [ -d \"$tmp/core-plugins\" ]; then ",
    "mkdir -p \"$HOME/.local/share/kumikit\"; ",
    "rm -rf \"$HOME/.local/share/kumikit/core-plugins\"; ",
    "mkdir -p \"$HOME/.local/share/kumikit/core-plugins\"; ",
    "cp -R \"$tmp/core-plugins/.\" \"$HOME/.local/share/kumikit/core-plugins/\"; ",
    "fi; ",
    "printf \"BIN=%s\\n\" \"$HOME/.local/bin/kumikit\"; ",
    "printf \"VER=%s\\n\" \"$(\"$HOME/.local/bin/kumikit\" --version 2>/dev/null | head -n 1 || true)\"",
    "'"
);

const START_CORE_SCRIPT: &str =
    "sh -c 'nohup \"$HOME/.local/bin/kumikit\" serve >/dev/null 2>&1 </dev/null &'";

struct ResolvedTarget {
    profile_name: Option<String>,
    destination: String,
    command: Option<String>,
}

fn resolve_target(
    profiles: &Profiles,
    req: &SshTargetRequest,
) -> Result<ResolvedTarget, CommandError> {
    if let Some(name) = req.profile.as_deref().filter(|s| !s.trim().is_empty()) {
        let profile = profiles.get(name)?;
        match profile.kind {
            Kind::Ssh {
                destination,
                command,
                ..
            } => Ok(ResolvedTarget {
                profile_name: Some(profile.name),
                destination,
                command,
            }),
            Kind::Tls { .. } => Err(CommandError::new(
                "invalid",
                format!("core `{name}` is not an SSH profile"),
            )),
        }
    } else if let Some(dest) = req.destination.as_deref().filter(|s| !s.trim().is_empty()) {
        ssh::check_destination(dest).map_err(|e| CommandError::new("invalid", e.to_string()))?;
        Ok(ResolvedTarget {
            profile_name: None,
            destination: dest.to_owned(),
            command: req.command.clone().filter(|c| !c.trim().is_empty()),
        })
    } else {
        Err(CommandError::new(
            "invalid",
            "provide either an SSH profile name or an SSH destination",
        ))
    }
}

/// Inspects a remote SSH host's OS, architecture, and installed `kumikit` version.
pub async fn probe(
    ssh_program: &Path,
    profiles: &Profiles,
    local_version: &str,
    req: &SshTargetRequest,
) -> Result<SshProbeResult, CommandError> {
    let target = resolve_target(profiles, req)?;
    let (stdout, _stderr) =
        run_ssh_capture(ssh_program, &target.destination, PROBE_SCRIPT, None).await?;
    Ok(parse_probe_output(&stdout, local_version))
}

/// Downloads and verifies the matching `kumikit` core archive locally, checks that remote
/// agents are idle before stopping an older core, streams the archive over SSH to
/// `$HOME/.local/bin/kumikit`, and starts the remote core if needed.
pub async fn sync(
    ssh_program: &Path,
    profiles: &Profiles,
    release: &ReleaseClient,
    client_dir: &Path,
    local_version: &str,
    req: &SshTargetRequest,
) -> Result<SshSyncResult, CommandError> {
    let resolved = resolve_target(profiles, req)?;
    let probed = probe(ssh_program, profiles, local_version, req).await?;
    let Some(target_triple) = probed.target.as_deref() else {
        return Err(CommandError::new(
            "unsupported",
            format!(
                "no Kumikit release binary is available for remote host {} {}",
                probed.os, probed.arch
            ),
        ));
    };

    let want_version = normalize_version(local_version).to_owned();
    let manifest = release
        .fetch_version(&want_version)
        .await
        .map_err(|e| CommandError::new("unavailable", e.to_string()))?;
    let Some(asset) = manifest.core.get(target_triple) else {
        return Err(CommandError::new(
            "unsupported",
            format!("release {want_version} has no core archive for {target_triple}"),
        ));
    };
    if asset.format != "tar.gz" {
        return Err(CommandError::new(
            "unsupported",
            format!(
                "SSH sync requires a tar.gz core archive, got {}",
                asset.format
            ),
        ));
    }

    // Download and verify SHA-256 locally (cached across multiple SSH hosts).
    let cache_path: PathBuf = client_dir
        .join("remote-cache")
        .join(format!("kumikit-v{want_version}-{target_triple}.tar.gz"));
    release
        .download_verified(asset, &cache_path)
        .await
        .map_err(|e| CommandError::new("untrusted", e.to_string()))?;
    let archive_bytes = tokio::fs::read(&cache_path)
        .await
        .map_err(|e| CommandError::new("internal", e.to_string()))?;

    // If an older remote binary exists, check if a core is running and shut it down cleanly
    // (respecting `if_idle` so active remote agents are never interrupted).
    if let Some(existing_bin) = probed.remote_binary.as_deref() {
        let bridge_cmd = resolved
            .command
            .clone()
            .unwrap_or_else(|| format!("{existing_bin} bridge"));
        shutdown_remote_core_if_running(
            ssh_program,
            &resolved.destination,
            &bridge_cmd,
            req.if_idle,
        )
        .await?;
    }

    // Stream the verified `.tar.gz` over SSH stdin to `$HOME/.local/bin/kumikit`.
    let (stdout, _stderr) = run_ssh_capture(
        ssh_program,
        &resolved.destination,
        INSTALL_STREAM_SCRIPT,
        Some(&archive_bytes),
    )
    .await?;
    let mut installed_bin = "$HOME/.local/bin/kumikit".to_owned();
    let mut installed_ver = want_version.clone();
    for line in stdout.lines() {
        if let Some(b) = line.strip_prefix("BIN=")
            && !b.trim().is_empty()
        {
            installed_bin = b.trim().to_owned();
        }
        if let Some(v) = line.strip_prefix("VER=")
            && let Some(parsed) = parse_cli_version(v)
        {
            installed_ver = parsed;
        }
    }

    // Ensure a core is running on the remote host.
    let mut started_core = false;
    if check_remote_bridge(ssh_program, &resolved.destination, LOCAL_BIN_BRIDGE_COMMAND)
        .await
        .is_err()
    {
        let _ =
            run_ssh_capture(ssh_program, &resolved.destination, START_CORE_SCRIPT, None).await?;
        started_core = true;
        let mut ready = false;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if check_remote_bridge(ssh_program, &resolved.destination, LOCAL_BIN_BRIDGE_COMMAND)
                .await
                .is_ok()
            {
                ready = true;
                break;
            }
        }
        if !ready {
            return Err(CommandError::new(
                "unreachable",
                "installed kumikit on the remote host, but `kumikit serve` did not become ready in time",
            ));
        }
    }

    // Non-interactive SSH shells often omit `$HOME/.local/bin` from `PATH`. Update default
    // command on saved profiles so reconnects always find the installed binary.
    let mut updated_command = None;
    if let Some(name) = resolved.profile_name.as_deref()
        && resolved.command.is_none()
    {
        let cmd = LOCAL_BIN_BRIDGE_COMMAND.to_owned();
        let _ = profiles.update_ssh(name, Some(Some(cmd.clone())), None);
        updated_command = Some(cmd);
    }

    Ok(SshSyncResult {
        installed_version: installed_ver,
        remote_binary: installed_bin,
        started_core,
        updated_command,
    })
}

/// Parses the `OS=`, `ARCH=`, `BIN=`, `VER=` lines returned by [`PROBE_SCRIPT`].
pub fn parse_probe_output(stdout: &str, local_version: &str) -> SshProbeResult {
    let mut os = String::new();
    let mut arch = String::new();
    let mut remote_binary = None;
    let mut remote_version = None;
    for line in stdout.lines() {
        if let Some(v) = line.strip_prefix("OS=") {
            os = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("ARCH=") {
            arch = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("BIN=") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                remote_binary = Some(trimmed.to_owned());
            }
        } else if let Some(v) = line.strip_prefix("VER=") {
            remote_version = parse_cli_version(v);
        }
    }
    let target = target_for_uname(&os, &arch).map(str::to_owned);
    let supported = target.is_some();
    let local = normalize_version(local_version).to_owned();
    let status = if !supported {
        "unsupported"
    } else if remote_binary.is_none() {
        "missing"
    } else if remote_version.as_deref() == Some(local.as_str()) {
        "match"
    } else {
        "mismatch"
    };
    SshProbeResult {
        os,
        arch,
        target,
        supported,
        remote_binary,
        remote_version,
        local_version: local,
        status,
    }
}

/// Extracts the version from `kumikit --version` (for example `"kumikit 0.1.0-m8"` -> `"0.1.0-m8"`).
pub fn parse_cli_version(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let token = trimmed.split_whitespace().last()?;
    let norm = normalize_version(token);
    if norm.is_empty() {
        None
    } else {
        Some(norm.to_owned())
    }
}

async fn run_ssh_capture(
    ssh_program: &Path,
    destination: &str,
    remote_cmd: &str,
    stdin_bytes: Option<&[u8]>,
) -> Result<(String, String), CommandError> {
    let args = ssh::args(destination, remote_cmd)
        .map_err(|e| CommandError::new("invalid", e.to_string()))?;
    let mut cmd = Command::new(ssh_program);
    cmd.args(args)
        .stdin(if stdin_bytes.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    ssh::configure_no_window(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| CommandError::new("unreachable", format!("cannot start ssh: {e}")))?;
    if let Some(bytes) = stdin_bytes
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin
            .write_all(bytes)
            .await
            .map_err(|e| CommandError::new("unreachable", format!("ssh write failed: {e}")))?;
        stdin.shutdown().await.map_err(|e| {
            CommandError::new("unreachable", format!("ssh close stdin failed: {e}"))
        })?;
    }
    let output = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| CommandError::new("unreachable", "ssh command timed out"))?
        .map_err(|e| CommandError::new("unreachable", e.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        let detail = if stderr.trim().is_empty() {
            format!("ssh exited with status {}", output.status)
        } else {
            format!("ssh: {}", stderr.trim())
        };
        return Err(CommandError::new("unreachable", detail));
    }
    Ok((stdout, stderr))
}

async fn check_remote_bridge(
    ssh_program: &Path,
    destination: &str,
    bridge_cmd: &str,
) -> Result<(), String> {
    let started = ssh::spawn(ssh_program, destination, bridge_cmd).map_err(|e| e.to_string())?;
    ssh::check(started.stdout, started.stdin, &started.stderr).await
}

/// Connects to an already-running remote core over SSH `bridge` and asks it to shut down
/// (`core.shutdown { ifIdle }`). If no core is running, returns `Ok(())`.
async fn shutdown_remote_core_if_running(
    ssh_program: &Path,
    destination: &str,
    bridge_cmd: &str,
    if_idle: bool,
) -> Result<(), CommandError> {
    let Ok(mut started) = ssh::spawn(ssh_program, destination, bridge_cmd) else {
        return Ok(());
    };
    let mut lines = BufReader::new(started.stdout).lines();

    // Try our protocol first; if the remote core speaks an older protocol, retry hello with
    // the protocol it announced so we can still cleanly query/shut it down.
    let mut protocol = PROTOCOL_VERSION.to_string();
    let mut authed = false;
    for attempt in 0..2 {
        let id = attempt + 1;
        let hello = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "core.hello",
            "params": {
                "protocol": protocol,
                "client": { "name": "kumikit-desktop", "version": APP_VERSION }
            }
        });
        if started
            .stdin
            .write_all(format!("{hello}\n").as_bytes())
            .await
            .is_err()
            || started.stdin.flush().await.is_err()
        {
            return Ok(());
        }
        let Ok(Ok(Some(line))) =
            tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await
        else {
            return Ok(());
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            return Ok(());
        };
        if v.get("result").is_some() {
            authed = true;
            break;
        }
        if let Some(msg) = v["error"]["message"].as_str()
            && let Some(ver) = msg.strip_prefix("core speaks protocol ")
        {
            protocol = ver.trim().to_owned();
            continue;
        }
        return Ok(());
    }
    if !authed {
        return Ok(());
    }

    let shutdown = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "core.shutdown",
        "params": { "ifIdle": if_idle }
    });
    if started
        .stdin
        .write_all(format!("{shutdown}\n").as_bytes())
        .await
        .is_err()
        || started.stdin.flush().await.is_err()
    {
        return Ok(());
    }
    while let Ok(Ok(Some(line))) =
        tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await
    {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v["id"] != 9 {
            continue;
        }
        if let Some(err) = v.get("error") {
            let code = err["data"]["code"].as_str().unwrap_or("internal");
            let msg = err["message"]
                .as_str()
                .unwrap_or("remote core refused shutdown");
            if code == "conflict" {
                return Err(CommandError::new("busy", msg));
            }
        }
        break;
    }
    // Brief pause so the remote core releases its socket before we start the new binary.
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_probe_outputs() {
        let missing = parse_probe_output("OS=Linux\nARCH=x86_64\nBIN=\nVER=\n", "0.1.0-m8");
        assert_eq!(missing.status, "missing");
        assert_eq!(missing.target.as_deref(), Some("x86_64-unknown-linux-gnu"));
        assert!(missing.supported);

        let matched = parse_probe_output(
            "OS=Linux\nARCH=aarch64\nBIN=/home/u/.local/bin/kumikit\nVER=kumikit 0.1.0-m8\n",
            "0.1.0-m8",
        );
        assert_eq!(matched.status, "match");
        assert_eq!(matched.remote_version.as_deref(), Some("0.1.0-m8"));

        let mismatch = parse_probe_output(
            "OS=Darwin\nARCH=arm64\nBIN=/usr/local/bin/kumikit\nVER=kumikit 0.1.0-m7.4\n",
            "0.1.0-m8",
        );
        assert_eq!(mismatch.status, "mismatch");
        assert_eq!(mismatch.target.as_deref(), Some("aarch64-apple-darwin"));

        let unsupported = parse_probe_output("OS=OpenBSD\nARCH=amd64\nBIN=\nVER=\n", "0.1.0-m8");
        assert_eq!(unsupported.status, "unsupported");
        assert!(!unsupported.supported);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ssh_sync_streams_verified_archive_to_remote_home() {
        use std::collections::BTreeMap;
        use std::os::unix::fs::PermissionsExt;

        use kumikit_package::signature::SigningKey;
        use kumikit_registry::fetch::Fetcher;
        use kumikit_registry::release::{
            RELEASE_MANIFEST_FORMAT, ReleaseAsset, ReleaseManifest, current_target,
            sha256_bytes_hex, sign_manifest,
        };

        let dir = tempfile::tempdir().unwrap();
        let remote_home = dir.path().join("remote-home");
        std::fs::create_dir_all(&remote_home).unwrap();
        let (key, _) = SigningKey::generate().unwrap();

        // Build a tiny fake `kumikit` executable script packed in a tar.gz.
        let archive = dir
            .path()
            .join("kumikit-v0.1.0-m8-x86_64-unknown-linux-gnu.tar.gz");
        {
            let f = std::fs::File::create(&archive).unwrap();
            let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            let mut tar = tar::Builder::new(enc);
            let script = br#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "kumikit 0.1.0-m8"
  exit 0
fi
if [ "$1" = "serve" ]; then
  touch "$HOME/.kumikit-served"
  exit 0
fi
if [ "$1" = "bridge" ]; then
  if [ -f "$HOME/.kumikit-served" ]; then
    read -r _line
    echo '{"jsonrpc":"2.0","id":0,"result":{"protocol":"0.10","core":{"version":"0.1.0-m8","name":"remote","os":"linux"},"plugins":[],"capabilities":[]}}'
    exit 0
  fi
  exit 1
fi
"#;
            let mut header = tar::Header::new_gnu();
            header.set_size(script.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, "kumikit", &script[..])
                .unwrap();
            let plugin_bytes = b"fake-kkp-payload";
            let mut pheader = tar::Header::new_gnu();
            pheader.set_size(plugin_bytes.len() as u64);
            pheader.set_mode(0o644);
            pheader.set_cksum();
            tar.append_data(
                &mut pheader,
                "core-plugins/example-0.1.0.kkp",
                &plugin_bytes[..],
            )
            .unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let archive_bytes = std::fs::read(&archive).unwrap();
        let mut core = BTreeMap::new();
        core.insert(
            current_target().to_owned(),
            ReleaseAsset {
                url: format!("file://{}", archive.display()),
                sha256: sha256_bytes_hex(&archive_bytes),
                size: archive_bytes.len() as u64,
                format: "tar.gz".into(),
            },
        );
        let manifest = ReleaseManifest {
            format: RELEASE_MANIFEST_FORMAT,
            version: "0.1.0-m8".into(),
            tag: "v0.1.0-m8".into(),
            published_at: "2026-11-25T00:00:00Z".into(),
            notes_url: "https://github.com/kumikit/kumikit/releases/tag/v0.1.0-m8".into(),
            notes: None,
            core,
            desktop: BTreeMap::new(),
        };
        let (json, sig) = sign_manifest(&manifest, &key).unwrap();
        let manifest_path = dir.path().join("release.json");
        std::fs::write(&manifest_path, &json).unwrap();
        std::fs::write(dir.path().join("release.json.sig"), &sig).unwrap();

        // Fake `ssh` executable: skips flags up to `--`, ignores destination, runs remote cmd in
        // an isolated HOME with clean PATH (so host kumikit isn't found).
        let fake_ssh = dir.path().join("fake-ssh");
        let ssh_script = format!(
            "#!/bin/sh\n\
             while [ \"$#\" -gt 0 ]; do\n\
               if [ \"$1\" = \"--\" ]; then shift; break; fi\n\
               shift\n\
             done\n\
             shift\n\
             HOME=\"{}\" PATH=\"/usr/bin:/bin\" exec sh -c \"$1\"\n",
            remote_home.display()
        );
        std::fs::write(&fake_ssh, ssh_script).unwrap();
        std::fs::set_permissions(&fake_ssh, std::fs::Permissions::from_mode(0o755)).unwrap();

        let profiles = Profiles::new(&dir.path().join("client"));
        let release = ReleaseClient::with_config(
            Fetcher::default(),
            format!("file://{}", manifest_path.display()),
            key.public(),
        );
        let req = SshTargetRequest {
            profile: None,
            destination: Some("devbox".into()),
            command: None,
            if_idle: true,
        };

        let before = probe(&fake_ssh, &profiles, "0.1.0-m8", &req).await.unwrap();
        assert_eq!(before.status, "missing");

        let synced = sync(
            &fake_ssh,
            &profiles,
            &release,
            &dir.path().join("client"),
            "0.1.0-m8",
            &req,
        )
        .await
        .unwrap();
        assert_eq!(synced.installed_version, "0.1.0-m8");
        assert!(synced.started_core);
        assert!(remote_home.join(".local/bin/kumikit").is_file());
        assert_eq!(
            std::fs::read(remote_home.join(".local/share/kumikit/core-plugins/example-0.1.0.kkp"))
                .unwrap(),
            b"fake-kkp-payload"
        );

        let after = probe(&fake_ssh, &profiles, "0.1.0-m8", &req).await.unwrap();
        assert_eq!(after.status, "match");
        assert_eq!(after.remote_version.as_deref(), Some("0.1.0-m8"));
    }

    #[test]
    fn remote_scripts_are_shell_portable_and_glob_free() {
        for script in [PROBE_SCRIPT, INSTALL_STREAM_SCRIPT, START_CORE_SCRIPT] {
            assert!(
                script.starts_with("sh -c '") && script.ends_with('\''),
                "remote script must run via POSIX `sh -c '...'`: {script}"
            );
            let inner = &script["sh -c '".len()..script.len() - 1];
            assert!(
                !inner.contains('\''),
                "single quotes inside `sh -c '...'` break non-POSIX login shells: {script}"
            );
            assert!(
                !inner.contains('*'),
                "remote script must not use shell globs (which fail under zsh NOMATCH): {script}"
            );
            assert!(
                ssh::check_command(script).is_ok(),
                "remote script must satisfy ssh::check_command: {script}"
            );
        }
    }
}
