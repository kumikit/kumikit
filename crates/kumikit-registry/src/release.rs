//! Signed Kumikit application and core release manifests
//! (`docs/spec/10-distribution.md#application-updates`,
//! [ADR 0031](../../../docs/decisions/0031-signed-release-manifest-and-updates.md)).
//!
//! Shared by the desktop shell (Settings → Updates and SSH remote sync), the core CLI
//! (`kumikit update`), and CI (`kumikit-registry release-manifest`). Every manifest is
//! verified against a pinned Ed25519 public key before any URL or SHA-256 hash inside it
//! is trusted.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;
#[cfg(feature = "archive")]
use std::path::Component;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kumikit_package::signature::{PublicKey, SigningKey, sign_file_bytes, verify_file_bytes};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fetch::{FetchError, Fetched, Fetcher, Limits};

/// Schema version of `release.json`.
pub const RELEASE_MANIFEST_FORMAT: u32 = 1;

/// Canonical file name signed inside `release.json.sig`.
pub const RELEASE_MANIFEST_NAME: &str = "release.json";

/// Default URL for the latest signed release manifest (works for both pre-1.0 milestone
/// releases and 1.0+ releases). Overridable via `KUMIKIT_UPDATE_URL`.
pub const DEFAULT_RELEASE_URL: &str =
    "https://raw.githubusercontent.com/kumikit/kumikit/release-manifest/release.json";

/// Default URL prefix for per-tag GitHub Release assets (`{prefix}{tag}/release.json`).
pub const DEFAULT_TAG_URL_PREFIX: &str = "https://github.com/kumikit/kumikit/releases/download/";

/// Pinned official Kumikit release signing public key. Overridable via
/// `KUMIKIT_UPDATE_KEY` for tests and custom builds.
pub const OFFICIAL_RELEASE_KEY: &str = "ed25519:00Ji-S866EV-IwRItk9Gk-M5N--4nvBzbbqvT6MRNNg";

/// Size cap for `release.json` and `release.json.sig` (1 MiB).
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// Size cap for downloaded installers and archives (256 MiB; AppImage bundles WebKitGTK).
pub const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;

/// Default time limits for large asset downloads.
pub fn asset_limits() -> Limits {
    Limits {
        connect: Duration::from_secs(15),
        request: Duration::from_secs(300),
    }
}

/// Errors while checking, verifying, downloading or extracting a release.
#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    /// Network or URL policy failure.
    #[error("{0}")]
    Fetch(#[from] FetchError),
    /// Missing, malformed, or invalid Ed25519 signature or SHA-256 mismatch.
    #[error("untrusted release: {0}")]
    Untrusted(String),
    /// Malformed manifest JSON, version string, or archive layout.
    #[error("invalid release: {0}")]
    Invalid(String),
    /// No release asset exists for the requested target or install kind.
    #[error("unsupported release target: {0}")]
    Unsupported(String),
    /// Filesystem I/O error.
    #[error("release I/O error: {0}")]
    Io(#[from] io::Error),
}

/// One downloadable binary archive or desktop installer listed in `release.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseAsset {
    /// Download URL (`https://`, loopback `http://`, or `file:` in tests).
    pub url: String,
    /// Lowercase hex SHA-256 digest of the file.
    pub sha256: String,
    /// File size in bytes.
    pub size: u64,
    /// Asset format: `tar.gz`, `zip`, `appimage`, `deb`, `rpm`, `app.tar.gz`, `dmg`, `nsis`.
    pub format: String,
}

/// Signed release manifest describing one Kumikit release across platforms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseManifest {
    /// Manifest format version (must be `1`).
    pub format: u32,
    /// Semver release version without leading `v` (for example `0.1.0-m8` or `1.0.0`).
    pub version: String,
    /// Git release tag (for example `v0.1.0-m8`).
    pub tag: String,
    /// RFC 3339 UTC timestamp when the manifest was produced.
    pub published_at: String,
    /// Human-readable release notes URL.
    pub notes_url: String,
    /// Optional short release summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Standalone `kumikit` core/CLI archives keyed by Rust target triple.
    #[serde(default)]
    pub core: BTreeMap<String, ReleaseAsset>,
    /// Desktop installers keyed by `<os>-<arch>-<kind>` (for example `linux-x86_64-appimage`,
    /// `linux-x86_64-deb`, `linux-x86_64-rpm`, `macos-aarch64-app`, `macos-aarch64-dmg`,
    /// `windows-x86_64-nsis`).
    #[serde(default)]
    pub desktop: BTreeMap<String, ReleaseAsset>,
}

/// Result of comparing a running version against the latest signed release manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    /// Currently running version (without leading `v`).
    pub current_version: String,
    /// Latest signed release version (without leading `v`).
    pub latest_version: String,
    /// True when `latest_version > current_version` by semver ordering.
    pub update_available: bool,
    /// The verified manifest.
    pub manifest: ReleaseManifest,
}

/// The Rust target triple of the current binary.
pub fn current_target() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        "unknown"
    }
}

/// Maps `uname -s` and `uname -m` from a remote Unix host to a Kumikit release target.
pub fn target_for_uname(os: &str, arch: &str) -> Option<&'static str> {
    match (os.trim(), arch.trim()) {
        ("Linux", "x86_64" | "amd64") => Some("x86_64-unknown-linux-gnu"),
        ("Linux", "aarch64" | "arm64") => Some("aarch64-unknown-linux-gnu"),
        ("Darwin", "arm64" | "aarch64") => Some("aarch64-apple-darwin"),
        ("Darwin", "x86_64") => Some("x86_64-apple-darwin"),
        _ => None,
    }
}

/// Normalizes a version string by trimming whitespace and a leading `v`.
pub fn normalize_version(version: &str) -> &str {
    let trimmed = version.trim();
    trimmed.strip_prefix('v').unwrap_or(trimmed)
}

/// Returns `true` when `candidate` is strictly newer than `current` under semver rules.
pub fn is_newer(current: &str, candidate: &str) -> Result<bool, ReleaseError> {
    let cur = semver::Version::parse(normalize_version(current))
        .map_err(|e| ReleaseError::Invalid(format!("bad current version `{current}`: {e}")))?;
    let next = semver::Version::parse(normalize_version(candidate))
        .map_err(|e| ReleaseError::Invalid(format!("bad release version `{candidate}`: {e}")))?;
    Ok(next > cur)
}

/// Sibling `.sig` URL for a `release.json` URL (inserted before any `?` or `#`).
pub fn signature_url(manifest_url: &str) -> String {
    let split = manifest_url.find(['?', '#']).unwrap_or(manifest_url.len());
    format!("{}.sig{}", &manifest_url[..split], &manifest_url[split..])
}

/// Configured manifest URL: `KUMIKIT_UPDATE_URL` or [`DEFAULT_RELEASE_URL`].
pub fn default_release_url(env: &dyn Fn(&str) -> Option<String>) -> String {
    env("KUMIKIT_UPDATE_URL")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_RELEASE_URL.to_owned())
}

/// Configured trusted public key: `KUMIKIT_UPDATE_KEY` or [`OFFICIAL_RELEASE_KEY`].
pub fn default_trusted_key(
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<PublicKey, ReleaseError> {
    let raw = env("KUMIKIT_UPDATE_KEY")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| OFFICIAL_RELEASE_KEY.to_owned());
    PublicKey::parse(&raw)
        .ok_or_else(|| ReleaseError::Invalid(format!("invalid release public key `{raw}`")))
}

/// Fetches, verifies, and downloads signed Kumikit releases.
#[derive(Debug, Clone)]
pub struct ReleaseClient {
    manifest_fetcher: Fetcher,
    asset_fetcher: Fetcher,
    manifest_url: String,
    tag_url_prefix: Option<String>,
    trusted_key: PublicKey,
}

impl ReleaseClient {
    /// Builds a release client from environment overrides (`KUMIKIT_UPDATE_URL`,
    /// `KUMIKIT_UPDATE_KEY`) or official defaults. Nothing is fetched until called.
    pub fn from_env(env: &dyn Fn(&str) -> Option<String>) -> Result<Self, ReleaseError> {
        let custom_url = env("KUMIKIT_UPDATE_URL").filter(|s| !s.trim().is_empty());
        let manifest_url = custom_url
            .clone()
            .unwrap_or_else(|| DEFAULT_RELEASE_URL.to_owned());
        let tag_url_prefix = if custom_url.is_some() {
            None
        } else {
            Some(DEFAULT_TAG_URL_PREFIX.to_owned())
        };
        let trusted_key = default_trusted_key(env)?;
        Ok(ReleaseClient {
            manifest_fetcher: Fetcher::new(Limits::default()),
            asset_fetcher: Fetcher::new(asset_limits()),
            manifest_url,
            tag_url_prefix,
            trusted_key,
        })
    }

    /// Builds a release client with explicit fetcher, manifest URL and trusted public key
    /// (used by tests and custom mirrors).
    pub fn with_config(
        fetcher: Fetcher,
        manifest_url: impl Into<String>,
        trusted_key: PublicKey,
    ) -> Self {
        ReleaseClient {
            manifest_fetcher: fetcher.clone(),
            asset_fetcher: fetcher,
            manifest_url: manifest_url.into(),
            tag_url_prefix: None,
            trusted_key,
        }
    }

    /// Overrides the per-tag URL prefix used by [`Self::fetch_version`].
    pub fn with_tag_url_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.tag_url_prefix = Some(prefix.into());
        self
    }

    /// The configured manifest URL.
    pub fn manifest_url(&self) -> &str {
        &self.manifest_url
    }

    /// The pinned public key.
    pub fn trusted_key(&self) -> PublicKey {
        self.trusted_key
    }

    /// Fetches and verifies a signed `release.json` at `url` (plus its sibling `.sig`).
    pub async fn fetch_url(&self, url: &str) -> Result<ReleaseManifest, ReleaseError> {
        let manifest_bytes = match self
            .manifest_fetcher
            .get(url, None, MAX_MANIFEST_BYTES)
            .await?
        {
            Fetched::Body { bytes, .. } => bytes,
            Fetched::NotModified => {
                return Err(ReleaseError::Invalid(
                    "unexpected 304 without cached manifest".into(),
                ));
            }
        };
        let sig_url = signature_url(url);
        let sig_bytes = match self
            .manifest_fetcher
            .get(&sig_url, None, MAX_MANIFEST_BYTES)
            .await
        {
            Ok(Fetched::Body { bytes, .. }) => bytes,
            Ok(Fetched::NotModified) => {
                return Err(ReleaseError::Untrusted(
                    "release signature missing (304)".into(),
                ));
            }
            Err(e) => {
                return Err(ReleaseError::Untrusted(format!(
                    "cannot fetch release signature ({sig_url}): {e}"
                )));
            }
        };
        let sig_text = String::from_utf8(sig_bytes)
            .map_err(|_| ReleaseError::Untrusted("release signature is not UTF-8".into()))?;
        verify_release_bytes(&manifest_bytes, &sig_text, self.trusted_key)
    }

    /// Fetches and verifies the latest signed release manifest.
    pub async fn fetch_latest(&self) -> Result<ReleaseManifest, ReleaseError> {
        self.fetch_url(&self.manifest_url).await
    }

    /// Fetches and verifies the signed release manifest for a specific `version` (used by
    /// SSH remote sync to install the exact matching version).
    pub async fn fetch_version(&self, version: &str) -> Result<ReleaseManifest, ReleaseError> {
        let norm = normalize_version(version);
        if let Some(prefix) = &self.tag_url_prefix {
            let tag_url = format!("{prefix}v{norm}/{RELEASE_MANIFEST_NAME}");
            match self.fetch_url(&tag_url).await {
                Ok(m) if normalize_version(&m.version) == norm => return Ok(m),
                Ok(m) => {
                    return Err(ReleaseError::Untrusted(format!(
                        "release manifest at {tag_url} declared version {}, expected {norm}",
                        m.version
                    )));
                }
                Err(first_err) => {
                    // Fall back to the configured manifest URL if it happens to be the requested version.
                    if let Ok(m) = self.fetch_latest().await
                        && normalize_version(&m.version) == norm
                    {
                        return Ok(m);
                    }
                    return Err(first_err);
                }
            }
        }
        let manifest = self.fetch_latest().await?;
        if normalize_version(&manifest.version) != norm {
            return Err(ReleaseError::Unsupported(format!(
                "manifest offers version {}, but matching version {norm} was requested",
                manifest.version
            )));
        }
        Ok(manifest)
    }

    /// Checks whether a newer signed release than `current_version` is available.
    pub async fn check_update(&self, current_version: &str) -> Result<UpdateCheck, ReleaseError> {
        let manifest = self.fetch_latest().await?;
        let current = normalize_version(current_version).to_owned();
        let latest = normalize_version(&manifest.version).to_owned();
        let update_available = is_newer(&current, &latest)?;
        Ok(UpdateCheck {
            current_version: current,
            latest_version: latest,
            update_available,
            manifest,
        })
    }

    /// Downloads `asset` to `dest_path`, verifying its SHA-256 digest against the signed
    /// manifest. If `dest_path` already exists with the expected SHA-256, it is reused
    /// without hitting the network.
    pub async fn download_verified(
        &self,
        asset: &ReleaseAsset,
        dest_path: &Path,
    ) -> Result<PathBuf, ReleaseError> {
        validate_sha256(&asset.sha256)?;
        if dest_path.is_file()
            && let Ok(existing) = sha256_file_hex(dest_path)
            && existing == asset.sha256
        {
            return Ok(dest_path.to_path_buf());
        }
        let bytes = match self
            .asset_fetcher
            .get(&asset.url, None, MAX_ASSET_BYTES)
            .await?
        {
            Fetched::Body { bytes, .. } => bytes,
            Fetched::NotModified => {
                return Err(ReleaseError::Invalid(
                    "unexpected 304 downloading release asset".into(),
                ));
            }
        };
        let actual = sha256_bytes_hex(&bytes);
        if actual != asset.sha256 {
            return Err(ReleaseError::Untrusted(format!(
                "SHA-256 mismatch for {}: expected {}, got {actual}",
                asset.url, asset.sha256
            )));
        }
        if let Some(parent) = dest_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let dest = dest_path.to_path_buf();
        tokio::task::spawn_blocking(move || write_atomic(&dest, &bytes))
            .await
            .map_err(|e| ReleaseError::Io(io::Error::other(e.to_string())))??;
        Ok(dest_path.to_path_buf())
    }
}

/// Verifies `manifest_bytes` against `signature` with `trusted_key` and parses/validates
/// the resulting [`ReleaseManifest`].
pub fn verify_release_bytes(
    manifest_bytes: &[u8],
    signature: &str,
    trusted_key: PublicKey,
) -> Result<ReleaseManifest, ReleaseError> {
    verify_file_bytes(
        RELEASE_MANIFEST_NAME,
        manifest_bytes,
        signature,
        Some(trusted_key),
    )
    .map_err(|e| ReleaseError::Untrusted(e.to_string()))?;
    let manifest: ReleaseManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| ReleaseError::Invalid(format!("cannot parse release.json: {e}")))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Serializes and signs a [`ReleaseManifest`], returning `(json_bytes, signature_text)`.
pub fn sign_manifest(
    manifest: &ReleaseManifest,
    key: &SigningKey,
) -> Result<(Vec<u8>, String), ReleaseError> {
    validate_manifest(manifest)?;
    let mut json = serde_json::to_vec_pretty(manifest)
        .map_err(|e| ReleaseError::Invalid(format!("cannot serialize release.json: {e}")))?;
    json.push(b'\n');
    let sig = sign_file_bytes(RELEASE_MANIFEST_NAME, &json, key)
        .map_err(|e| ReleaseError::Invalid(e.to_string()))?;
    Ok((json, sig))
}

/// Validates a parsed [`ReleaseManifest`].
pub fn validate_manifest(manifest: &ReleaseManifest) -> Result<(), ReleaseError> {
    if manifest.format != RELEASE_MANIFEST_FORMAT {
        return Err(ReleaseError::Invalid(format!(
            "unsupported release manifest format {} (expected {RELEASE_MANIFEST_FORMAT})",
            manifest.format
        )));
    }
    semver::Version::parse(normalize_version(&manifest.version)).map_err(|e| {
        ReleaseError::Invalid(format!(
            "invalid release version `{}`: {e}",
            manifest.version
        ))
    })?;
    if manifest.tag.trim().is_empty() {
        return Err(ReleaseError::Invalid("release tag is empty".into()));
    }
    for (k, asset) in manifest.core.iter().chain(manifest.desktop.iter()) {
        validate_sha256(&asset.sha256).map_err(|_| {
            ReleaseError::Invalid(format!("asset `{k}` has invalid sha256 `{}`", asset.sha256))
        })?;
        if asset.url.trim().is_empty() {
            return Err(ReleaseError::Invalid(format!("asset `{k}` has empty url")));
        }
    }
    Ok(())
}

/// Builds a [`ReleaseManifest`] by scanning staged release assets in `assets_dir`.
pub fn build_manifest_from_dir(
    tag: &str,
    assets_dir: &Path,
    base_url: &str,
    published_at: &str,
    notes_url: &str,
    notes: Option<String>,
) -> Result<ReleaseManifest, ReleaseError> {
    let version = normalize_version(tag).to_owned();
    semver::Version::parse(&version)
        .map_err(|e| ReleaseError::Invalid(format!("tag `{tag}` is not valid semver: {e}")))?;
    let base = base_url.trim_end_matches('/');
    let mut core = BTreeMap::new();
    let mut desktop = BTreeMap::new();

    for entry in std::fs::read_dir(assets_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let meta = std::fs::metadata(&path)?;
        let size = meta.len();

        let core_prefix = format!("kumikit-{tag}-");
        if let Some(rest) = name.strip_prefix(&core_prefix) {
            if let Some(target) = rest.strip_suffix(".tar.gz") {
                core.insert(
                    target.to_owned(),
                    ReleaseAsset {
                        url: format!("{base}/{name}"),
                        sha256: sha256_file_hex(&path)?,
                        size,
                        format: "tar.gz".into(),
                    },
                );
                continue;
            }
            if let Some(target) = rest.strip_suffix(".zip") {
                core.insert(
                    target.to_owned(),
                    ReleaseAsset {
                        url: format!("{base}/{name}"),
                        sha256: sha256_file_hex(&path)?,
                        size,
                        format: "zip".into(),
                    },
                );
                continue;
            }
        }

        let desktop_prefix = format!("Kumikit-{tag}-");
        if let Some(rest) = name.strip_prefix(&desktop_prefix) {
            let mapping: Option<(&str, &str)> = match rest {
                "x86_64.AppImage" => Some(("linux-x86_64-appimage", "appimage")),
                "x86_64.deb" => Some(("linux-x86_64-deb", "deb")),
                "x86_64.rpm" => Some(("linux-x86_64-rpm", "rpm")),
                "aarch64.app.tar.gz" => Some(("macos-aarch64-app", "app.tar.gz")),
                "aarch64.dmg" => Some(("macos-aarch64-dmg", "dmg")),
                "x86_64-setup.exe" => Some(("windows-x86_64-nsis", "nsis")),
                _ => None,
            };
            if let Some((key, format)) = mapping {
                desktop.insert(
                    key.to_owned(),
                    ReleaseAsset {
                        url: format!("{base}/{name}"),
                        sha256: sha256_file_hex(&path)?,
                        size,
                        format: format.into(),
                    },
                );
            }
        }
    }

    let manifest = ReleaseManifest {
        format: RELEASE_MANIFEST_FORMAT,
        version,
        tag: tag.to_owned(),
        published_at: published_at.to_owned(),
        notes_url: notes_url.to_owned(),
        notes,
        core,
        desktop,
    };
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Extracted contents of a verified core release archive.
#[cfg(feature = "archive")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedCore {
    /// Path to the extracted `kumikit` (or `kumikit.exe`) binary.
    pub binary: PathBuf,
    /// Paths to any bundled `.kkp` files extracted from `core-plugins/`.
    pub core_plugins: Vec<PathBuf>,
}

/// Extracts the `kumikit` (or `kumikit.exe`) binary from a verified core archive into
/// `dest_dir`, refusing path traversal or symlinks.
#[cfg(feature = "archive")]
pub fn extract_core_binary(
    archive: &Path,
    format: &str,
    dest_dir: &Path,
) -> Result<PathBuf, ReleaseError> {
    extract_core_bundle(archive, format, dest_dir).map(|b| b.binary)
}

/// Extracts the `kumikit` binary and any bundled `core-plugins/*.kkp` packages from a
/// verified core archive into `dest_dir`, refusing path traversal or symlinks.
#[cfg(feature = "archive")]
pub fn extract_core_bundle(
    archive: &Path,
    format: &str,
    dest_dir: &Path,
) -> Result<ExtractedCore, ReleaseError> {
    std::fs::create_dir_all(dest_dir)?;
    let bin_name = if cfg!(windows) {
        "kumikit.exe"
    } else {
        "kumikit"
    };
    match format {
        "tar.gz" => {
            let file = std::fs::File::open(archive)?;
            let gz = flate2::read::GzDecoder::new(file);
            let mut tar = tar::Archive::new(gz);
            let mut binary = None;
            let mut core_plugins = Vec::new();
            for entry in tar.entries()? {
                let mut entry = entry?;
                let path = entry
                    .path()
                    .map_err(|e| ReleaseError::Invalid(format!("bad archive path: {e}")))?
                    .into_owned();
                if path.components().any(|c| {
                    matches!(
                        c,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                }) {
                    return Err(ReleaseError::Untrusted(format!(
                        "unsafe path in release archive: {}",
                        path.display()
                    )));
                }
                if !entry.header().entry_type().is_file() {
                    continue;
                }
                let normal: Vec<&std::ffi::OsStr> = path
                    .components()
                    .filter_map(|c| match c {
                        Component::Normal(s) => Some(s),
                        _ => None,
                    })
                    .collect();
                if normal.len() == 1 && normal[0] == bin_name {
                    let out = dest_dir.join(bin_name);
                    let mut f = std::fs::File::create(&out)?;
                    io::copy(&mut entry, &mut f)?;
                    f.sync_all()?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755))?;
                    }
                    binary = Some(out);
                } else if normal.len() == 2
                    && normal[0] == "core-plugins"
                    && Path::new(normal[1]).extension().is_some_and(|e| e == "kkp")
                {
                    let plugins_dir = dest_dir.join("core-plugins");
                    std::fs::create_dir_all(&plugins_dir)?;
                    let out = plugins_dir.join(normal[1]);
                    let mut f = std::fs::File::create(&out)?;
                    io::copy(&mut entry, &mut f)?;
                    f.sync_all()?;
                    core_plugins.push(out);
                }
            }
            core_plugins.sort();
            let binary = binary.ok_or_else(|| {
                ReleaseError::Invalid(format!("{bin_name} was not found in {}", archive.display()))
            })?;
            Ok(ExtractedCore {
                binary,
                core_plugins,
            })
        }
        "zip" => {
            // Windows 10+ ships bsdtar as `tar.exe`, which extracts `.zip` archives cleanly.
            let status = std::process::Command::new("tar")
                .arg("-xf")
                .arg(archive)
                .arg("-C")
                .arg(dest_dir)
                .status()?;
            if !status.success() {
                return Err(ReleaseError::Invalid(format!(
                    "failed to extract {}",
                    archive.display()
                )));
            }
            let binary = dest_dir.join(bin_name);
            if !binary.is_file() {
                return Err(ReleaseError::Invalid(format!(
                    "{bin_name} was not found in {}",
                    archive.display()
                )));
            }
            let mut core_plugins = Vec::new();
            let plugins_dir = dest_dir.join("core-plugins");
            if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() && p.extension().is_some_and(|e| e == "kkp") {
                        core_plugins.push(p);
                    }
                }
            }
            core_plugins.sort();
            Ok(ExtractedCore {
                binary,
                core_plugins,
            })
        }
        other => Err(ReleaseError::Unsupported(format!(
            "unsupported core archive format `{other}`"
        ))),
    }
}

/// Computes the lowercase hex SHA-256 digest of a slice.
pub fn sha256_bytes_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Computes the lowercase hex SHA-256 digest of a file.
pub fn sha256_file_hex(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(&mut out, "{b:02x}");
    }
    Ok(out)
}

fn validate_sha256(hex: &str) -> Result<(), ReleaseError> {
    if hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(ReleaseError::Invalid(format!(
            "invalid SHA-256 digest `{hex}`"
        )))
    }
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(dest).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest(version: &str, asset_url: &str, asset_bytes: &[u8]) -> ReleaseManifest {
        let mut core = BTreeMap::new();
        core.insert(
            current_target().to_owned(),
            ReleaseAsset {
                url: asset_url.to_owned(),
                sha256: sha256_bytes_hex(asset_bytes),
                size: asset_bytes.len() as u64,
                format: "tar.gz".into(),
            },
        );
        ReleaseManifest {
            format: RELEASE_MANIFEST_FORMAT,
            version: version.to_owned(),
            tag: format!("v{version}"),
            published_at: "2026-11-25T00:00:00Z".into(),
            notes_url: format!("https://github.com/kumikit/kumikit/releases/tag/v{version}"),
            notes: Some("Test release".into()),
            core,
            desktop: BTreeMap::new(),
        }
    }

    #[test]
    fn semver_ordering_handles_milestone_prereleases() {
        assert!(is_newer("0.1.0-m7", "0.1.0-m7.4").unwrap());
        assert!(is_newer("0.1.0-m7.4", "0.1.0-m8").unwrap());
        assert!(is_newer("0.1.0-m8", "0.1.0").unwrap());
        assert!(is_newer("v0.1.0", "v0.2.0").unwrap());
        assert!(!is_newer("0.1.0-m8", "0.1.0-m8").unwrap());
        assert!(!is_newer("0.1.0-m8", "0.1.0-m7.4").unwrap());
    }

    #[test]
    fn target_mapping_from_uname() {
        assert_eq!(
            target_for_uname("Linux", "x86_64"),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(
            target_for_uname("Linux", "aarch64"),
            Some("aarch64-unknown-linux-gnu")
        );
        assert_eq!(
            target_for_uname("Darwin", "arm64"),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(target_for_uname("FreeBSD", "x86_64"), None);
    }

    #[test]
    fn signature_verification_rejects_tampering_and_wrong_key() {
        let (key, _) = SigningKey::generate().unwrap();
        let (other_key, _) = SigningKey::generate().unwrap();
        let manifest = sample_manifest("0.1.0-m8", "file:///tmp/x.tar.gz", b"payload");
        let (json, sig) = sign_manifest(&manifest, &key).unwrap();

        let parsed = verify_release_bytes(&json, &sig, key.public()).unwrap();
        assert_eq!(parsed, manifest);

        // Wrong key is rejected.
        assert!(matches!(
            verify_release_bytes(&json, &sig, other_key.public()),
            Err(ReleaseError::Untrusted(_))
        ));

        // Tampered JSON is rejected.
        let mut tampered = json.clone();
        tampered[0] = b' ';
        assert!(matches!(
            verify_release_bytes(&tampered, &sig, key.public()),
            Err(ReleaseError::Untrusted(_))
        ));
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
    async fn client_fetches_verifies_and_downloads_from_file_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let (key, _) = SigningKey::generate().unwrap();

        let archive_path = dir.path().join("kumikit-v0.1.0-m8.tar.gz");
        #[cfg(feature = "archive")]
        {
            let f = std::fs::File::create(&archive_path).unwrap();
            let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            let mut tar = tar::Builder::new(enc);
            let body = b"#!/bin/sh\necho kumikit 0.1.0-m8\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            let name = if cfg!(windows) {
                "kumikit.exe"
            } else {
                "kumikit"
            };
            tar.append_data(&mut header, name, &body[..]).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        #[cfg(not(feature = "archive"))]
        std::fs::write(&archive_path, b"fake-archive-bytes").unwrap();

        let archive_bytes = std::fs::read(&archive_path).unwrap();
        let archive_url = file_url(&archive_path);
        let manifest = sample_manifest("0.1.0-m8", &archive_url, &archive_bytes);
        let (json, sig) = sign_manifest(&manifest, &key).unwrap();
        let manifest_path = dir.path().join("release.json");
        std::fs::write(&manifest_path, &json).unwrap();
        std::fs::write(dir.path().join("release.json.sig"), &sig).unwrap();

        let manifest_url = file_url(&manifest_path);
        let client = ReleaseClient::with_config(Fetcher::default(), manifest_url, key.public());
        let check = client.check_update("0.1.0-m7.4").await.unwrap();
        assert!(check.update_available);
        assert_eq!(check.latest_version, "0.1.0-m8");

        let asset = check.manifest.core.get(current_target()).unwrap();
        let dest = dir.path().join("cache/archive.tar.gz");
        client.download_verified(asset, &dest).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), archive_bytes);
        #[cfg(feature = "archive")]
        {
            let extracted =
                extract_core_binary(&dest, &asset.format, &dir.path().join("extract")).unwrap();
            assert!(extracted.is_file());
        }

        // Bad hash fails verification.
        let mut bad_asset = asset.clone();
        bad_asset.sha256 = "0".repeat(64);
        assert!(matches!(
            client
                .download_verified(&bad_asset, &dir.path().join("cache/bad.tar.gz"))
                .await,
            Err(ReleaseError::Untrusted(_))
        ));
    }

    #[test]
    fn build_manifest_scans_staged_assets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path()
                .join("kumikit-v0.1.0-m8-x86_64-unknown-linux-gnu.tar.gz"),
            b"core-linux",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Kumikit-v0.1.0-m8-x86_64.AppImage"),
            b"appimage",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Kumikit-v0.1.0-m8-aarch64.app.tar.gz"),
            b"mac-app",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Kumikit-v0.1.0-m8-x86_64-setup.exe"),
            b"win-nsis",
        )
        .unwrap();

        let manifest = build_manifest_from_dir(
            "v0.1.0-m8",
            dir.path(),
            "https://github.com/kumikit/kumikit/releases/download/v0.1.0-m8",
            "2026-11-25T00:00:00Z",
            "https://github.com/kumikit/kumikit/releases/tag/v0.1.0-m8",
            None,
        )
        .unwrap();

        assert_eq!(manifest.version, "0.1.0-m8");
        assert!(manifest.core.contains_key("x86_64-unknown-linux-gnu"));
        assert!(manifest.desktop.contains_key("linux-x86_64-appimage"));
        assert!(manifest.desktop.contains_key("macos-aarch64-app"));
        assert!(manifest.desktop.contains_key("windows-x86_64-nsis"));
    }
}
