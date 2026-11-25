//! Desktop application updates (`docs/spec/06-client.md#shell-rust-kumikit-shell`,
//! [ADR 0031](../../../docs/decisions/0031-signed-release-manifest-and-updates.md)).
//!
//! Fetches and verifies the Ed25519-signed `release.json` manifest, stages verified
//! platform installers in `<client data>/updates/`, guards local core shutdown with
//! `core.shutdown { ifIdle }` so active agents are never interrupted, and applies updates
//! in-place on AppImage, macOS `.app`, and Windows NSIS (or hands off the verified package
//! path on `.deb`/`.rpm` system installs).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kumikit_protocol::{APP_VERSION, PROTOCOL_VERSION};
use kumikit_registry::release::{
    ReleaseAsset, ReleaseClient, ReleaseError, ReleaseManifest, current_target, normalize_version,
    sha256_file_hex,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::connections::CommandError;
use crate::transport;

/// How the desktop app is installed on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedInstall {
    /// Linux AppImage at a writable path (supports atomic self-replacement and relaunch).
    AppImage(PathBuf),
    /// Linux system package (`.deb`); download and verify only.
    Deb,
    /// Linux system package (`.rpm`); download and verify only.
    Rpm,
    /// macOS `.app` bundle at a writable path (supports in-place swap from `.app.tar.gz`).
    MacApp(PathBuf),
    /// Windows NSIS installation (supports silent `-setup.exe /S` upgrade).
    WindowsNsis,
    /// Development build or read-only/unrecognized layout.
    Unsupported,
}

impl DetectedInstall {
    /// Stable wire identifier for the UI.
    pub fn kind_str(&self) -> &'static str {
        match self {
            DetectedInstall::AppImage(_) => "appimage",
            DetectedInstall::Deb => "deb",
            DetectedInstall::Rpm => "rpm",
            DetectedInstall::MacApp(_) => "macos_app",
            DetectedInstall::WindowsNsis => "windows_nsis",
            DetectedInstall::Unsupported => "unsupported",
        }
    }

    /// Key in `ReleaseManifest::desktop` for this install kind, if any.
    pub fn desktop_asset_key(&self) -> Option<&'static str> {
        match self {
            DetectedInstall::AppImage(_) => Some("linux-x86_64-appimage"),
            DetectedInstall::Deb => Some("linux-x86_64-deb"),
            DetectedInstall::Rpm => Some("linux-x86_64-rpm"),
            DetectedInstall::MacApp(_) => Some("macos-aarch64-app"),
            DetectedInstall::WindowsNsis => Some("windows-x86_64-nsis"),
            DetectedInstall::Unsupported => None,
        }
    }

    /// Whether the shell can replace the running app and relaunch automatically.
    pub fn can_auto_install(&self) -> bool {
        matches!(
            self,
            DetectedInstall::AppImage(_)
                | DetectedInstall::MacApp(_)
                | DetectedInstall::WindowsNsis
        )
    }
}

/// Detects how the desktop app was installed (`KUMIKIT_UPDATE_INSTALL_KIND` overrides in tests).
pub fn detect_install(
    env: &dyn Fn(&str) -> Option<String>,
    current_exe: Option<&Path>,
) -> DetectedInstall {
    if let Some(raw) = env("KUMIKIT_UPDATE_INSTALL_KIND") {
        let raw = raw.trim();
        if let Some(p) = raw.strip_prefix("appimage:") {
            return DetectedInstall::AppImage(PathBuf::from(p));
        }
        if let Some(p) = raw.strip_prefix("macos_app:") {
            return DetectedInstall::MacApp(PathBuf::from(p));
        }
        return match raw {
            "deb" => DetectedInstall::Deb,
            "rpm" => DetectedInstall::Rpm,
            "windows_nsis" => DetectedInstall::WindowsNsis,
            _ => DetectedInstall::Unsupported,
        };
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(appimage) = env("APPIMAGE") {
            let p = PathBuf::from(appimage);
            if p.is_file() && dir_is_writable(p.parent()) {
                return DetectedInstall::AppImage(p);
            }
        }
        if let Some(exe) = current_exe {
            let s = exe.to_string_lossy();
            if s.starts_with("/usr/") {
                if Path::new("/var/lib/dpkg/status").is_file() {
                    return DetectedInstall::Deb;
                }
                if Path::new("/var/lib/rpm").is_dir() || Path::new("/usr/bin/rpm").is_file() {
                    return DetectedInstall::Rpm;
                }
            }
        }
        DetectedInstall::Unsupported
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(exe) = current_exe {
            for ancestor in exe.ancestors() {
                if ancestor.extension().and_then(|e| e.to_str()) == Some("app")
                    && dir_is_writable(ancestor.parent())
                {
                    return DetectedInstall::MacApp(ancestor.to_path_buf());
                }
            }
        }
        DetectedInstall::Unsupported
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(exe) = current_exe {
            let s = exe.to_string_lossy().to_ascii_lowercase();
            if !s.contains("\\target\\debug\\") && !s.contains("\\target\\release\\") {
                return DetectedInstall::WindowsNsis;
            }
        }
        DetectedInstall::Unsupported
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = current_exe;
        DetectedInstall::Unsupported
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn dir_is_writable(dir: Option<&Path>) -> bool {
    let Some(dir) = dir else { return false };
    tempfile::NamedTempFile::new_in(dir).is_ok()
}

/// Static info about this app build (`app_update_info`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Current stamped version (without leading `v`).
    pub current_version: String,
    /// Target triple (`x86_64-unknown-linux-gnu`, etc.).
    pub target: String,
    /// Install kind (`appimage`, `deb`, `rpm`, `macos_app`, `windows_nsis`, `unsupported`).
    pub install_kind: &'static str,
    /// Whether this installation can be updated and relaunched automatically.
    pub can_auto_install: bool,
    /// Version already downloaded and verified in `<client data>/updates/`, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staged_version: Option<String>,
}

/// Result of `app_update_check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    /// Current version.
    pub current_version: String,
    /// Latest signed release version.
    pub latest_version: String,
    /// True when `latest_version > current_version`.
    pub update_available: bool,
    /// Release tag (for example `v0.1.0-m8`).
    pub tag: String,
    /// Release publication timestamp (RFC 3339).
    pub published_at: String,
    /// Link to release notes.
    pub notes_url: String,
    /// Optional short summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Install kind of this app.
    pub install_kind: &'static str,
    /// Whether in-place auto-install is supported here.
    pub can_auto_install: bool,
    /// Whether the manifest contains an installer asset for this platform/kind.
    pub asset_available: bool,
    /// Size of the downloadable installer asset in bytes, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_size: Option<u64>,
}

/// Result of `app_update_download`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDownloadResult {
    /// Downloaded version.
    pub version: String,
    /// Local path of the verified installer file.
    pub path: String,
    /// Whether this installation supports automatic in-place apply.
    pub can_auto_install: bool,
    /// True when verified and staged.
    pub ready_to_install: bool,
}

/// Request for `app_update_apply`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApplyRequest {
    /// Version to apply (must have been downloaded & verified).
    pub version: String,
    /// When true, refuse with `busy` if any agent turn is running or queued on the local core.
    #[serde(default = "default_true")]
    pub if_idle: bool,
}

fn default_true() -> bool {
    true
}

/// Result of `app_update_apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApplyResult {
    /// True when the update was applied in place.
    pub applied: bool,
    /// True when the app is exiting and relaunching into the new version.
    pub relaunching: bool,
    /// Human-readable status or manual install instruction.
    pub message: String,
    /// Path to the verified package for package-managed installs (`.deb`/`.rpm`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagedRecord {
    version: String,
    file_name: String,
    sha256: String,
    format: String,
}

/// Manages signed app update checks, downloads, and idle-gated installs for `kumikit-shell`.
#[derive(Debug, Clone)]
pub struct AppUpdater {
    updates_dir: PathBuf,
    local_core_address: String,
    current_version: String,
    target: String,
    detected: DetectedInstall,
    release: Option<ReleaseClient>,
    release_err: Option<String>,
    last_manifest: Arc<Mutex<Option<ReleaseManifest>>>,
    /// True in tests and `web-relay` so `apply` never spawns or kills the test process.
    skip_relaunch: bool,
}

impl AppUpdater {
    /// Creates an updater from environment overrides (`KUMIKIT_UPDATE_URL`,
    /// `KUMIKIT_UPDATE_KEY`, `KUMIKIT_UPDATE_INSTALL_KIND`). Does zero network I/O.
    pub fn from_env(
        client_dir: &Path,
        local_core_address: String,
        env: &dyn Fn(&str) -> Option<String>,
        skip_relaunch: bool,
    ) -> Self {
        let exe = std::env::current_exe().ok();
        let detected = detect_install(env, exe.as_deref());
        let (release, release_err) = match ReleaseClient::from_env(env) {
            Ok(c) => (Some(c), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let current_version = env("KUMIKIT_TEST_APP_VERSION")
            .filter(|s| !s.trim().is_empty())
            .map(|s| normalize_version(&s).to_owned())
            .unwrap_or_else(|| normalize_version(APP_VERSION).to_owned());
        AppUpdater {
            updates_dir: client_dir.join("updates"),
            local_core_address,
            current_version,
            target: current_target().to_owned(),
            detected,
            release,
            release_err,
            last_manifest: Arc::new(Mutex::new(None)),
            skip_relaunch,
        }
    }

    /// Creates an updater with an explicit [`ReleaseClient`] and install kind (tests).
    pub fn with_client(
        client_dir: &Path,
        local_core_address: String,
        current_version: impl Into<String>,
        detected: DetectedInstall,
        release: ReleaseClient,
    ) -> Self {
        AppUpdater {
            updates_dir: client_dir.join("updates"),
            local_core_address,
            current_version: normalize_version(&current_version.into()).to_owned(),
            target: current_target().to_owned(),
            detected,
            release: Some(release),
            release_err: None,
            last_manifest: Arc::new(Mutex::new(None)),
            skip_relaunch: true,
        }
    }

    /// The underlying release client (reused by SSH remote sync).
    pub fn release_client(&self) -> Result<&ReleaseClient, CommandError> {
        self.release.as_ref().ok_or_else(|| {
            CommandError::new(
                "invalid",
                self.release_err
                    .clone()
                    .unwrap_or_else(|| "release client is not configured".into()),
            )
        })
    }

    /// Current stamped version of the desktop app.
    pub fn current_version(&self) -> &str {
        &self.current_version
    }

    /// Static app and staging info (`app_update_info`).
    pub fn info(&self) -> UpdateInfo {
        UpdateInfo {
            current_version: self.current_version.clone(),
            target: self.target.clone(),
            install_kind: self.detected.kind_str(),
            can_auto_install: self.detected.can_auto_install(),
            staged_version: self.latest_staged_version(),
        }
    }

    /// Fetches and verifies the latest signed release manifest (`app_update_check`).
    pub async fn check(&self) -> Result<UpdateCheckResult, CommandError> {
        let client = self.release_client()?;
        let check = client
            .check_update(&self.current_version)
            .await
            .map_err(release_err)?;
        let asset = self.pick_desktop_asset(&check.manifest);
        let result = UpdateCheckResult {
            current_version: check.current_version,
            latest_version: check.latest_version,
            update_available: check.update_available,
            tag: check.manifest.tag.clone(),
            published_at: check.manifest.published_at.clone(),
            notes_url: check.manifest.notes_url.clone(),
            notes: check.manifest.notes.clone(),
            install_kind: self.detected.kind_str(),
            can_auto_install: self.detected.can_auto_install() && asset.is_some(),
            asset_available: asset.is_some(),
            asset_size: asset.map(|a| a.size),
        };
        *self.last_manifest.lock().unwrap_or_else(|p| p.into_inner()) = Some(check.manifest);
        Ok(result)
    }

    /// Downloads and SHA-256-verifies the latest release installer (`app_update_download`).
    pub async fn download(&self) -> Result<UpdateDownloadResult, CommandError> {
        let client = self.release_client()?;
        let manifest = {
            let cached = self
                .last_manifest
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            match cached {
                Some(m) => m,
                None => {
                    let check = client
                        .check_update(&self.current_version)
                        .await
                        .map_err(release_err)?;
                    *self.last_manifest.lock().unwrap_or_else(|p| p.into_inner()) =
                        Some(check.manifest.clone());
                    check.manifest
                }
            }
        };
        let Some(asset) = self.pick_desktop_asset(&manifest) else {
            return Err(CommandError::new(
                "unsupported",
                format!(
                    "release {} does not provide an installer for {} ({})",
                    manifest.version,
                    self.target,
                    self.detected.kind_str()
                ),
            ));
        };
        let file_name = asset
            .url
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty() && !s.contains(".."))
            .unwrap_or("kumikit-update")
            .to_owned();
        let ver_dir = self.updates_dir.join(&manifest.version);
        let dest = ver_dir.join(&file_name);
        client
            .download_verified(asset, &dest)
            .await
            .map_err(release_err)?;
        let record = StagedRecord {
            version: manifest.version.clone(),
            file_name,
            sha256: asset.sha256.clone(),
            format: asset.format.clone(),
        };
        let meta_bytes = serde_json::to_vec_pretty(&record)
            .map_err(|e| CommandError::new("internal", e.to_string()))?;
        tokio::fs::write(ver_dir.join("staged.json"), meta_bytes)
            .await
            .map_err(|e| CommandError::new("internal", e.to_string()))?;
        Ok(UpdateDownloadResult {
            version: manifest.version,
            path: dest.to_string_lossy().into_owned(),
            can_auto_install: self.detected.can_auto_install(),
            ready_to_install: true,
        })
    }

    /// Applies a previously downloaded and verified update (`app_update_apply`).
    /// When `if_idle` is true and any local agent turn or ready queued message is active,
    /// returns `code: "busy"` without stopping the core or touching the app.
    pub async fn apply(&self, req: UpdateApplyRequest) -> Result<UpdateApplyResult, CommandError> {
        let version = normalize_version(&req.version).to_owned();
        let ver_dir = self.updates_dir.join(&version);
        let record_bytes = tokio::fs::read(ver_dir.join("staged.json"))
            .await
            .map_err(|_| {
                CommandError::new(
                    "not_found",
                    format!("update {version} has not been downloaded yet"),
                )
            })?;
        let record: StagedRecord = serde_json::from_slice(&record_bytes)
            .map_err(|e| CommandError::new("invalid", e.to_string()))?;
        let staged_path = ver_dir.join(&record.file_name);
        let actual = sha256_file_hex(&staged_path).map_err(|e| {
            CommandError::new("internal", format!("cannot read staged update: {e}"))
        })?;
        if actual != record.sha256 {
            return Err(CommandError::new(
                "untrusted",
                "the staged update file failed SHA-256 verification; download it again",
            ));
        }

        match &self.detected {
            DetectedInstall::Deb => {
                let p = staged_path.to_string_lossy().into_owned();
                return Ok(UpdateApplyResult {
                    applied: false,
                    relaunching: false,
                    message: format!(
                        "Verified .deb package downloaded to {p}. Install it with: sudo apt install \"{p}\""
                    ),
                    package_path: Some(p),
                });
            }
            DetectedInstall::Rpm => {
                let p = staged_path.to_string_lossy().into_owned();
                return Ok(UpdateApplyResult {
                    applied: false,
                    relaunching: false,
                    message: format!(
                        "Verified .rpm package downloaded to {p}. Install it with: sudo dnf install \"{p}\""
                    ),
                    package_path: Some(p),
                });
            }
            DetectedInstall::Unsupported => {
                return Err(CommandError::new(
                    "unsupported",
                    "in-place updates are not supported for this installation; download the release from GitHub",
                ));
            }
            DetectedInstall::AppImage(_)
            | DetectedInstall::MacApp(_)
            | DetectedInstall::WindowsNsis => {}
        }

        // Stop the local core cleanly first (refuses with `busy` if `if_idle` and agents are working).
        shutdown_local_core(&self.local_core_address, req.if_idle).await?;

        match &self.detected {
            DetectedInstall::AppImage(appimage_path) => {
                let target = appimage_path.clone();
                let src = staged_path.clone();
                tokio::task::spawn_blocking(move || replace_executable(&src, &target))
                    .await
                    .map_err(|e| CommandError::new("internal", e.to_string()))?
                    .map_err(|e| {
                        CommandError::new("internal", format!("cannot replace AppImage: {e}"))
                    })?;
                if !self.skip_relaunch {
                    spawn_relaunch_appimage(appimage_path);
                }
                Ok(UpdateApplyResult {
                    applied: true,
                    relaunching: !self.skip_relaunch,
                    message: format!("Updated Kumikit to {version}."),
                    package_path: None,
                })
            }
            DetectedInstall::MacApp(app_bundle) => {
                let bundle = app_bundle.clone();
                let src = staged_path.clone();
                tokio::task::spawn_blocking(move || replace_macos_app(&src, &bundle))
                    .await
                    .map_err(|e| CommandError::new("internal", e.to_string()))?
                    .map_err(|e| {
                        CommandError::new("internal", format!("cannot update macOS app: {e}"))
                    })?;
                if !self.skip_relaunch {
                    spawn_relaunch_macos(app_bundle);
                }
                Ok(UpdateApplyResult {
                    applied: true,
                    relaunching: !self.skip_relaunch,
                    message: format!("Updated Kumikit to {version}."),
                    package_path: None,
                })
            }
            DetectedInstall::WindowsNsis => {
                if !self.skip_relaunch {
                    spawn_windows_nsis(&staged_path).map_err(|e| {
                        CommandError::new("internal", format!("cannot launch installer: {e}"))
                    })?;
                }
                Ok(UpdateApplyResult {
                    applied: true,
                    relaunching: !self.skip_relaunch,
                    message: format!("Installing Kumikit {version}…"),
                    package_path: None,
                })
            }
            DetectedInstall::Deb | DetectedInstall::Rpm | DetectedInstall::Unsupported => {
                unreachable!("handled above")
            }
        }
    }

    fn pick_desktop_asset<'a>(&self, manifest: &'a ReleaseManifest) -> Option<&'a ReleaseAsset> {
        let key = self.detected.desktop_asset_key()?;
        manifest.desktop.get(key)
    }

    fn latest_staged_version(&self) -> Option<String> {
        let entries = std::fs::read_dir(&self.updates_dir).ok()?;
        let mut best: Option<(semver::Version, String)> = None;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.join("staged.json").is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Ok(ver) = semver::Version::parse(normalize_version(name))
                && best.as_ref().is_none_or(|(b, _)| ver > *b)
            {
                best = Some((ver, name.to_owned()));
            }
        }
        best.map(|(_, s)| s)
    }
}

fn release_err(e: ReleaseError) -> CommandError {
    let code = match &e {
        ReleaseError::Fetch(_) => "unavailable",
        ReleaseError::Untrusted(_) => "untrusted",
        ReleaseError::Invalid(_) => "invalid",
        ReleaseError::Unsupported(_) => "unsupported",
        ReleaseError::Io(_) => "internal",
    };
    CommandError::new(code, e.to_string())
}

/// Sends `core.shutdown { ifIdle }` to the local core if one is running.
pub async fn shutdown_local_core(address: &str, if_idle: bool) -> Result<bool, CommandError> {
    let Ok(stream) = transport::connect(address).await else {
        return Ok(false);
    };
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();

    let hello = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "core.hello",
        "params": {
            "protocol": PROTOCOL_VERSION.to_string(),
            "client": { "name": "kumikit-desktop", "version": APP_VERSION }
        }
    });
    write
        .write_all(format!("{hello}\n").as_bytes())
        .await
        .map_err(|e| CommandError::new("internal", e.to_string()))?;
    write
        .flush()
        .await
        .map_err(|e| CommandError::new("internal", e.to_string()))?;
    let _ = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .map_err(|_| CommandError::new("unreachable", "local core did not answer core.hello"))?
        .map_err(|e| CommandError::new("internal", e.to_string()))?;

    let shutdown = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "core.shutdown",
        "params": { "ifIdle": if_idle }
    });
    write
        .write_all(format!("{shutdown}\n").as_bytes())
        .await
        .map_err(|e| CommandError::new("internal", e.to_string()))?;
    write
        .flush()
        .await
        .map_err(|e| CommandError::new("internal", e.to_string()))?;

    while let Ok(Ok(Some(line))) =
        tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await
    {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v["id"] != 2 {
            continue;
        }
        if let Some(err) = v.get("error") {
            let code = err["data"]["code"].as_str().unwrap_or("internal");
            let msg = err["message"]
                .as_str()
                .unwrap_or("local core refused shutdown");
            if code == "conflict" {
                return Err(CommandError::new("busy", msg));
            }
            return Err(CommandError::new("internal", msg));
        }
        break;
    }
    drop(write);
    drop(lines);
    for _ in 0..40 {
        if transport::connect(address).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(true)
}

fn replace_executable(src: &Path, dst: &Path) -> std::io::Result<()> {
    let parent = dst
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    let mut input = std::fs::File::open(src)?;
    std::io::copy(&mut input, tmp.as_file_mut())?;
    tmp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755))?;
    }
    tmp.persist(dst).map_err(|e| e.error)?;
    Ok(())
}

fn replace_macos_app(tar_gz: &Path, app_bundle: &Path) -> std::io::Result<()> {
    let parent = app_bundle
        .parent()
        .unwrap_or_else(|| Path::new("/Applications"));
    let stage = tempfile::tempdir_in(parent)?;
    let file = std::fs::File::open(tar_gz)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive.unpack(stage.path())?;
    let mut extracted_app = None;
    for entry in std::fs::read_dir(stage.path())? {
        let entry = entry?;
        if entry.path().extension().and_then(|e| e.to_str()) == Some("app") {
            extracted_app = Some(entry.path());
            break;
        }
    }
    let Some(new_app) = extracted_app else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "no .app bundle found in update archive",
        ));
    };
    let backup = parent.join(format!(".kumikit-prev-{}.app", std::process::id()));
    let _ = std::fs::remove_dir_all(&backup);
    if app_bundle.exists() {
        std::fs::rename(app_bundle, &backup)?;
    }
    if let Err(e) = std::fs::rename(&new_app, app_bundle) {
        let _ = std::fs::rename(&backup, app_bundle);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

fn spawn_relaunch_appimage(appimage_path: &Path) {
    let path = appimage_path.to_path_buf();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        let _ = std::process::Command::new(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        std::process::exit(0);
    });
}

fn spawn_relaunch_macos(app_bundle: &Path) {
    let path = app_bundle.to_path_buf();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        let _ = std::process::Command::new("open")
            .arg("-n")
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        std::process::exit(0);
    });
}

fn spawn_windows_nsis(setup_exe: &Path) -> std::io::Result<()> {
    let path = setup_exe.to_path_buf();
    std::process::Command::new(&path)
        .arg("/S")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(400));
        std::process::exit(0);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use kumikit_package::signature::SigningKey;
    use kumikit_registry::fetch::Fetcher;
    use kumikit_registry::release::{RELEASE_MANIFEST_FORMAT, sha256_bytes_hex, sign_manifest};

    use super::*;

    #[tokio::test]
    async fn appimage_check_download_and_atomic_apply() {
        let dir = tempfile::tempdir().unwrap();
        let (key, _) = SigningKey::generate().unwrap();

        let fake_appimage = dir.path().join("Kumikit.AppImage");
        std::fs::write(&fake_appimage, b"old-appimage").unwrap();

        let new_payload = b"new-verified-appimage-bytes";
        let asset_path = dir.path().join("Kumikit-v0.1.0-m8-x86_64.AppImage");
        std::fs::write(&asset_path, new_payload).unwrap();

        let mut desktop = BTreeMap::new();
        desktop.insert(
            "linux-x86_64-appimage".into(),
            ReleaseAsset {
                url: format!("file://{}", asset_path.display()),
                sha256: sha256_bytes_hex(new_payload),
                size: new_payload.len() as u64,
                format: "appimage".into(),
            },
        );
        let manifest = ReleaseManifest {
            format: RELEASE_MANIFEST_FORMAT,
            version: "0.1.0-m8".into(),
            tag: "v0.1.0-m8".into(),
            published_at: "2026-11-25T00:00:00Z".into(),
            notes_url: "https://github.com/kumikit/kumikit/releases/tag/v0.1.0-m8".into(),
            notes: Some("M8 updates".into()),
            core: BTreeMap::new(),
            desktop,
        };
        let (json, sig) = sign_manifest(&manifest, &key).unwrap();
        let manifest_path = dir.path().join("release.json");
        std::fs::write(&manifest_path, &json).unwrap();
        std::fs::write(dir.path().join("release.json.sig"), &sig).unwrap();

        let client = ReleaseClient::with_config(
            Fetcher::default(),
            format!("file://{}", manifest_path.display()),
            key.public(),
        );
        let updater = AppUpdater::with_client(
            &dir.path().join("client"),
            dir.path()
                .join("no-core.sock")
                .to_string_lossy()
                .into_owned(),
            "0.1.0-m7.4",
            DetectedInstall::AppImage(fake_appimage.clone()),
            client,
        );

        let check = updater.check().await.unwrap();
        assert!(check.update_available);
        assert!(check.can_auto_install);
        assert_eq!(check.latest_version, "0.1.0-m8");

        let dl = updater.download().await.unwrap();
        assert!(dl.ready_to_install);
        assert_eq!(updater.info().staged_version.as_deref(), Some("0.1.0-m8"));

        let applied = updater
            .apply(UpdateApplyRequest {
                version: "0.1.0-m8".into(),
                if_idle: true,
            })
            .await
            .unwrap();
        assert!(applied.applied);
        assert_eq!(std::fs::read(&fake_appimage).unwrap(), new_payload);
    }

    #[tokio::test]
    async fn deb_install_downloads_verified_package_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let (key, _) = SigningKey::generate().unwrap();
        let deb_bytes = b"fake-deb-package";
        let deb_path = dir.path().join("Kumikit-v0.1.0-m8-x86_64.deb");
        std::fs::write(&deb_path, deb_bytes).unwrap();

        let mut desktop = BTreeMap::new();
        desktop.insert(
            "linux-x86_64-deb".into(),
            ReleaseAsset {
                url: format!("file://{}", deb_path.display()),
                sha256: sha256_bytes_hex(deb_bytes),
                size: deb_bytes.len() as u64,
                format: "deb".into(),
            },
        );
        let manifest = ReleaseManifest {
            format: RELEASE_MANIFEST_FORMAT,
            version: "0.1.0-m8".into(),
            tag: "v0.1.0-m8".into(),
            published_at: "2026-11-25T00:00:00Z".into(),
            notes_url: "https://github.com/kumikit/kumikit/releases/tag/v0.1.0-m8".into(),
            notes: None,
            core: BTreeMap::new(),
            desktop,
        };
        let (json, sig) = sign_manifest(&manifest, &key).unwrap();
        let manifest_path = dir.path().join("release.json");
        std::fs::write(&manifest_path, &json).unwrap();
        std::fs::write(dir.path().join("release.json.sig"), &sig).unwrap();

        let client = ReleaseClient::with_config(
            Fetcher::default(),
            format!("file://{}", manifest_path.display()),
            key.public(),
        );
        let updater = AppUpdater::with_client(
            &dir.path().join("client"),
            dir.path()
                .join("no-core.sock")
                .to_string_lossy()
                .into_owned(),
            "0.1.0-m7.4",
            DetectedInstall::Deb,
            client,
        );
        let check = updater.check().await.unwrap();
        assert!(check.update_available);
        assert!(!check.can_auto_install);
        assert!(check.asset_available);

        updater.download().await.unwrap();
        let applied = updater
            .apply(UpdateApplyRequest {
                version: "0.1.0-m8".into(),
                if_idle: true,
            })
            .await
            .unwrap();
        assert!(!applied.applied);
        assert!(applied.package_path.is_some());
        assert!(applied.message.contains("apt install"));
    }
}
