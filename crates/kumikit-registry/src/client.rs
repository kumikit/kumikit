//! Reading registries on demand, with a cache (`docs/spec/10-distribution.md#fetching`).
//!
//! Nothing here runs unless the user asked for something: a search reuses an index younger
//! than [`MAX_AGE`]; installing and checking for updates revalidate it with its ETag.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::fetch::{FetchError, Fetched, Fetcher, file_path, is_loopback};
use crate::index::{Index, Version, parse_index};

/// The official registry's index.
pub const OFFICIAL: &str = "https://kumikit.github.io/registry/index.json";

/// The environment variable that replaces the default registry (tests, development).
pub const DEFAULT_ENV: &str = "KUMIKIT_DEFAULT_REGISTRY";

/// How long a search may reuse a cached index.
pub const MAX_AGE: Duration = Duration::from_secs(10 * 60);

/// Largest index read.
pub const MAX_INDEX: u64 = 16 << 20;

/// Largest package downloaded.
pub const MAX_PACKAGE: u64 = 64 << 20;

/// A registry operation that failed, by the protocol code it maps to.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// Not reachable, or a bad answer (`unavailable`).
    #[error("{0}")]
    Unavailable(String),
    /// A download does not match the index (`untrusted`).
    #[error("{0}")]
    Untrusted(String),
    /// A bad URL or request (`invalid_params`).
    #[error("{0}")]
    Invalid(String),
    /// Not listed (`not_found`).
    #[error("{0}")]
    NotFound(String),
}

impl From<FetchError> for RegistryError {
    fn from(e: FetchError) -> Self {
        match e {
            FetchError::Refused(m) => RegistryError::Invalid(m),
            FetchError::Unavailable(m) | FetchError::TooLarge(m) => RegistryError::Unavailable(m),
        }
    }
}

/// Whether a registry URL is one Kumikit reads: `https`, `http` to a loopback host, or
/// `file:`.
pub fn check_url(url: &str) -> Result<(), RegistryError> {
    if file_path(url).is_some() {
        return Ok(());
    }
    let uri: hyper::Uri = url
        .parse()
        .map_err(|_| RegistryError::Invalid(format!("{url} is not a URL")))?;
    let host = uri.host().unwrap_or_default();
    match uri.scheme_str() {
        Some("https") if !host.is_empty() => Ok(()),
        Some("http") if is_loopback(host) => Ok(()),
        _ => Err(RegistryError::Invalid(format!(
            "{url}: a registry must be an https:// URL (http:// only on this computer, or file:)"
        ))),
    }
}

/// The default registry: `KUMIKIT_DEFAULT_REGISTRY`, or the official one.
pub fn default_registry(env: &dyn Fn(&str) -> Option<String>) -> String {
    env(DEFAULT_ENV).unwrap_or_else(|| OFFICIAL.to_owned())
}

/// The ordered list of registries, kept in a small TOML file (`registries.toml`).
#[derive(Debug, Clone)]
pub struct RegistryList {
    path: PathBuf,
    default: String,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ListFile {
    #[serde(default)]
    registries: Vec<String>,
}

impl RegistryList {
    /// The list in `path`; when the file does not exist, the list is just `default`.
    pub fn new(path: PathBuf, default: String) -> RegistryList {
        RegistryList { path, default }
    }

    /// The registries, in resolution order.
    pub async fn get(&self) -> Result<Vec<String>, RegistryError> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(text) => toml::from_str::<ListFile>(&text)
                .map(|f| f.registries)
                .map_err(|e| {
                    RegistryError::Invalid(format!("{} is malformed: {e}", self.path.display()))
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![self.default.clone()]),
            Err(e) => Err(RegistryError::Unavailable(format!(
                "cannot read {}: {e}",
                self.path.display()
            ))),
        }
    }

    /// Replaces the list. Every URL is checked; duplicates are dropped.
    pub async fn set(&self, urls: &[String]) -> Result<(), RegistryError> {
        let mut out: Vec<String> = Vec::new();
        for u in urls {
            check_url(u)?;
            if !out.contains(u) {
                out.push(u.clone());
            }
        }
        let text = toml::to_string(&ListFile { registries: out })
            .map_err(|e| RegistryError::Invalid(e.to_string()))?;
        let text =
            format!("# The plugin registries this side reads, in order (10-distribution).\n{text}");
        write_atomic(&self.path, text.as_bytes())
            .await
            .map_err(|e| {
                RegistryError::Unavailable(format!("cannot write {}: {e}", self.path.display()))
            })
    }

    /// Appends a registry.
    pub async fn add(&self, url: &str) -> Result<(), RegistryError> {
        let mut list = self.get().await?;
        list.push(url.to_owned());
        self.set(&list).await
    }

    /// Removes a registry.
    pub async fn remove(&self, url: &str) -> Result<(), RegistryError> {
        let list: Vec<String> = self.get().await?.into_iter().filter(|u| u != url).collect();
        self.set(&list).await
    }
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await
}

/// How fresh an index must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// A cached copy younger than [`MAX_AGE`] is fine (browsing, searching). If the
    /// registry cannot be reached, an older copy is used.
    Cached,
    /// Ask the registry (with the ETag, so an unchanged index is not sent again).
    Revalidate,
}

/// An index and when it was fetched.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedIndex {
    /// The registry's URL.
    pub url: String,
    /// The index.
    pub index: Index,
    /// When it was last fetched or confirmed.
    pub fetched: SystemTime,
}

/// Reads registries through a cache directory.
#[derive(Clone)]
pub struct RegistryClient {
    fetcher: Fetcher,
    cache: PathBuf,
    now: Arc<dyn Fn() -> SystemTime + Send + Sync>,
}

impl std::fmt::Debug for RegistryClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryClient")
            .field("cache", &self.cache)
            .finish()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl RegistryClient {
    /// A client caching in `cache` (created on first use).
    pub fn new(fetcher: Fetcher, cache: PathBuf) -> RegistryClient {
        RegistryClient {
            fetcher,
            cache,
            now: Arc::new(SystemTime::now),
        }
    }

    /// The same client with another clock (tests).
    pub fn with_clock(mut self, now: Arc<dyn Fn() -> SystemTime + Send + Sync>) -> RegistryClient {
        self.now = now;
        self
    }

    fn cached(&self, url: &str) -> PathBuf {
        self.cache.join(hex(&Sha256::digest(url.as_bytes())))
    }

    /// The index at `url`.
    pub async fn index(
        &self,
        url: &str,
        freshness: Freshness,
    ) -> Result<FetchedIndex, RegistryError> {
        check_url(url)?;
        let base = self.cached(url);
        let (json, etag, meta) = (
            base.with_extension("json"),
            base.with_extension("etag"),
            base.with_extension("time"),
        );
        let cached = tokio::fs::read(&json).await.ok();
        let cached_etag = tokio::fs::read_to_string(&etag).await.ok();
        let cached_time = tokio::fs::read_to_string(&meta)
            .await
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok())
            .map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s));
        let now = (self.now)();
        if freshness == Freshness::Cached
            && let (Some(bytes), Some(t)) = (&cached, cached_time)
            && now.duration_since(t).is_ok_and(|age| age < MAX_AGE)
            && let Ok(index) = parse_index(bytes)
        {
            return Ok(FetchedIndex {
                url: url.to_owned(),
                index,
                fetched: t,
            });
        }
        let etag_to_send = cached.as_ref().and(cached_etag.as_deref());
        let answer = self.fetcher.get(url, etag_to_send, MAX_INDEX).await;
        let bytes = match answer {
            Ok(Fetched::NotModified) => match cached {
                Some(b) => b,
                None => {
                    return Err(RegistryError::Unavailable(format!(
                        "{url} answered 'not modified' to a request without a cached copy"
                    )));
                }
            },
            Ok(Fetched::Body { bytes, etag: new }) => {
                let _ = tokio::fs::create_dir_all(&self.cache).await;
                let _ = write_atomic(&json, &bytes).await;
                match new {
                    Some(e) => {
                        let _ = write_atomic(&etag, e.as_bytes()).await;
                    }
                    None => {
                        let _ = tokio::fs::remove_file(&etag).await;
                    }
                }
                bytes
            }
            Err(e) => {
                // Browsing may show an old copy when the registry is unreachable.
                if freshness == Freshness::Cached
                    && let (Some(bytes), Some(t)) = (&cached, cached_time)
                    && let Ok(index) = parse_index(bytes)
                {
                    tracing::info!(registry = %url, error = %e, "using a cached registry index");
                    return Ok(FetchedIndex {
                        url: url.to_owned(),
                        index,
                        fetched: t,
                    });
                }
                return Err(e.into());
            }
        };
        let index =
            parse_index(&bytes).map_err(|m| RegistryError::Unavailable(format!("{url}: {m}")))?;
        let secs = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let _ = write_atomic(&meta, secs.to_string().as_bytes()).await;
        Ok(FetchedIndex {
            url: url.to_owned(),
            index,
            fetched: now,
        })
    }

    /// Downloads a version's package and checks it against the index's SHA-256 (and
    /// `expected`, from a lockfile). Returns the bytes.
    pub async fn download(
        &self,
        version: &Version,
        expected: Option<&str>,
    ) -> Result<Vec<u8>, RegistryError> {
        let bytes = match self.fetcher.get(&version.url, None, MAX_PACKAGE).await? {
            Fetched::Body { bytes, .. } => bytes,
            Fetched::NotModified => {
                return Err(RegistryError::Unavailable(format!(
                    "{} answered 'not modified' to a plain request",
                    version.url
                )));
            }
        };
        let got = hex(&Sha256::digest(&bytes));
        if !got.eq_ignore_ascii_case(&version.sha256) {
            return Err(RegistryError::Untrusted(format!(
                "the download from {} does not match the registry's SHA-256",
                version.url
            )));
        }
        if let Some(want) = expected
            && !got.eq_ignore_ascii_case(want)
        {
            return Err(RegistryError::Untrusted(format!(
                "the package's SHA-256 differs from the lockfile's ({want})"
            )));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_urls() {
        for ok in [
            "https://kumikit.github.io/registry/index.json",
            "http://127.0.0.1:9/index.json",
            "http://localhost/index.json",
            "file:///srv/registry/index.json",
        ] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://example.com/index.json",
            "ftp://x/y",
            "index.json",
            "https:///x",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn the_registry_list_defaults_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let list = RegistryList::new(dir.path().join("registries.toml"), OFFICIAL.into());
        assert_eq!(list.get().await.unwrap(), [OFFICIAL]);
        list.add("http://127.0.0.1:1/index.json").await.unwrap();
        list.add("http://127.0.0.1:1/index.json").await.unwrap();
        assert_eq!(
            list.get().await.unwrap(),
            [OFFICIAL, "http://127.0.0.1:1/index.json"]
        );
        assert!(list.add("http://example.com/i.json").await.is_err());
        list.remove(OFFICIAL).await.unwrap();
        assert_eq!(list.get().await.unwrap(), ["http://127.0.0.1:1/index.json"]);
        list.set(&[]).await.unwrap();
        assert!(
            list.get().await.unwrap().is_empty(),
            "an emptied list stays empty"
        );
    }

    #[test]
    fn the_default_can_be_replaced() {
        assert_eq!(default_registry(&|_| None), OFFICIAL);
        let env = |k: &str| (k == DEFAULT_ENV).then(|| "http://127.0.0.1:5/i.json".to_owned());
        assert_eq!(default_registry(&env), "http://127.0.0.1:5/i.json");
    }
}
