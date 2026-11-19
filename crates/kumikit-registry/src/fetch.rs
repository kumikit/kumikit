//! Fetching registry indexes and packages (`docs/spec/10-distribution.md#fetching`).
//!
//! - `https://` is verified with the operating system's trusted certificates, loaded on the
//!   first HTTPS fetch (nothing is read at startup).
//! - `http://` is allowed only to a loopback host, for tests and local mirrors.
//! - `file:` reads a file on this machine.
//!
//! Redirects are followed up to [`MAX_REDIRECTS`] times, only to `https` (or from loopback
//! `http` to loopback `http`). Every fetch has a size cap and time limits.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper::{Request, StatusCode, Uri, header};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// Most redirects followed for one fetch.
pub const MAX_REDIRECTS: usize = 5;

/// Time limits for one fetch.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// To open the TCP connection (each hop).
    pub connect: Duration,
    /// For the whole fetch, redirects included.
    pub request: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            connect: Duration::from_secs(10),
            request: Duration::from_secs(30),
        }
    }
}

/// Why a fetch failed, in words for the user.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The URL is not one Kumikit fetches from (scheme, host or redirect rule).
    #[error("{0}")]
    Refused(String),
    /// The server could not be reached, or answered with an error.
    #[error("{0}")]
    Unavailable(String),
    /// The body is over the size cap.
    #[error("{0}")]
    TooLarge(String),
}

/// The result of a fetch.
#[derive(Debug, PartialEq, Eq)]
pub enum Fetched {
    /// The server says the cached copy (by its ETag) is current.
    NotModified,
    /// The body, and its ETag if the server sent one.
    Body {
        /// The bytes.
        bytes: Vec<u8>,
        /// The `ETag` header.
        etag: Option<String>,
    },
}

/// Fetches over HTTPS, loopback HTTP or from files. Cheap to clone.
#[derive(Clone)]
pub struct Fetcher {
    tls: Arc<OnceLock<Result<Arc<rustls::ClientConfig>, String>>>,
    limits: Limits,
}

impl std::fmt::Debug for Fetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fetcher")
            .field("limits", &self.limits)
            .finish()
    }
}

impl Default for Fetcher {
    fn default() -> Self {
        Fetcher::new(Limits::default())
    }
}

impl Fetcher {
    /// A fetcher that trusts the operating system's certificates.
    pub fn new(limits: Limits) -> Fetcher {
        Fetcher {
            tls: Arc::new(OnceLock::new()),
            limits,
        }
    }

    /// A fetcher that trusts only `roots` (tests use their own certificate authority).
    pub fn with_roots(roots: rustls::RootCertStore, limits: Limits) -> Fetcher {
        let cell = OnceLock::new();
        let _ = cell.set(Ok(client_config(roots)));
        Fetcher {
            tls: Arc::new(cell),
            limits,
        }
    }

    /// Fetches `url`. With `etag`, a server that still has that version answers
    /// [`Fetched::NotModified`]. Bodies over `max` bytes fail with [`FetchError::TooLarge`].
    pub async fn get(
        &self,
        url: &str,
        etag: Option<&str>,
        max: u64,
    ) -> Result<Fetched, FetchError> {
        if let Some(path) = file_path(url) {
            return read_file(path, max).await;
        }
        let uri = parse(url)?;
        check_scheme(&uri, None)?;
        match tokio::time::timeout(self.limits.request, self.follow(uri, etag, max)).await {
            Ok(result) => result,
            Err(_) => Err(FetchError::Unavailable(format!(
                "{} did not answer within {} s",
                host_of(url),
                self.limits.request.as_secs()
            ))),
        }
    }

    async fn follow(
        &self,
        mut uri: Uri,
        etag: Option<&str>,
        max: u64,
    ) -> Result<Fetched, FetchError> {
        for _ in 0..=MAX_REDIRECTS {
            match self.once(&uri, etag, max).await? {
                Hop::Done(fetched) => return Ok(fetched),
                Hop::Redirect(location) => {
                    let next = resolve(&uri, &location)?;
                    check_scheme(&next, Some(&uri))?;
                    tracing::debug!(from = %uri, to = %next, "registry redirect");
                    uri = next;
                }
            }
        }
        Err(FetchError::Unavailable(format!(
            "more than {MAX_REDIRECTS} redirects from {}",
            uri.host().unwrap_or_default()
        )))
    }

    async fn once(&self, uri: &Uri, etag: Option<&str>, max: u64) -> Result<Hop, FetchError> {
        let host = uri.host().unwrap_or_default().to_owned();
        let https = uri.scheme_str() == Some("https");
        let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
        // Bracketed IPv6 literals come back from `host()` with their brackets.
        let connect_host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        // Trust roots first, so a broken system store is reported as such.
        let config = if https {
            Some(self.tls_config().await?)
        } else {
            None
        };
        let tcp = tokio::time::timeout(
            self.limits.connect,
            TcpStream::connect((connect_host.as_str(), port)),
        )
        .await
        .map_err(|_| FetchError::Unavailable(format!("cannot reach {host}: timed out")))?
        .map_err(|e| FetchError::Unavailable(format!("cannot reach {host}: {e}")))?;
        if let Some(config) = config {
            let name = ServerName::try_from(connect_host.clone())
                .map_err(|_| FetchError::Refused(format!("{host} is not a valid host name")))?;
            let tls = TlsConnector::from(config)
                .connect(name, tcp)
                .await
                .map_err(|e| FetchError::Unavailable(format!("TLS with {host} failed: {e}")))?;
            request(tls, uri, etag, max).await
        } else {
            request(tcp, uri, etag, max).await
        }
    }

    async fn tls_config(&self) -> Result<Arc<rustls::ClientConfig>, FetchError> {
        if let Some(done) = self.tls.get() {
            return done.clone().map_err(FetchError::Unavailable);
        }
        // Reading the system store touches files, so it runs off the async threads.
        let loaded = tokio::task::spawn_blocking(load_native_roots)
            .await
            .map_err(|e| FetchError::Unavailable(e.to_string()))?;
        let _ = self.tls.set(loaded.map(client_config));
        self.tls
            .get()
            .cloned()
            .unwrap_or_else(|| Err("no TLS configuration".into()))
            .map_err(FetchError::Unavailable)
    }
}

enum Hop {
    Done(Fetched),
    Redirect(String),
}

fn load_native_roots() -> Result<rustls::RootCertStore, String> {
    let found = rustls_native_certs::load_native_certs();
    let mut roots = rustls::RootCertStore::empty();
    let (added, _ignored) = roots.add_parsable_certificates(found.certs);
    if added == 0 {
        let why = found
            .errors
            .first()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "none found".into());
        return Err(format!(
            "cannot load this system's trusted certificates: {why}"
        ));
    }
    Ok(roots)
}

fn client_config(roots: rustls::RootCertStore) -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    // The ring provider supports the default protocol versions, so this cannot fail.
    #[allow(clippy::expect_used)]
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}

async fn request<S>(stream: S, uri: &Uri, etag: Option<&str>, max: u64) -> Result<Hop, FetchError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let host = uri.host().unwrap_or_default().to_owned();
    let unavailable = |e: hyper::Error| FetchError::Unavailable(format!("{host}: {e}"));
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(unavailable)?;
    let driver = tokio::spawn(async move {
        let _ = conn.await;
    });
    let authority = uri
        .authority()
        .map(|a| a.as_str().to_owned())
        .unwrap_or_default();
    let path = uri
        .path_and_query()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "/".into());
    let mut builder = Request::get(path)
        .header(header::HOST, authority)
        .header(
            header::USER_AGENT,
            concat!("kumikit/", env!("CARGO_PKG_VERSION")),
        )
        .header(header::ACCEPT_ENCODING, "identity");
    if let Some(etag) = etag {
        builder = builder.header(header::IF_NONE_MATCH, etag);
    }
    let req = builder
        .body(Empty::<Bytes>::new())
        .map_err(|e| FetchError::Refused(e.to_string()))?;
    let response = sender.send_request(req).await.map_err(unavailable)?;
    let status = response.status();
    let result = if status == StatusCode::NOT_MODIFIED {
        Ok(Hop::Done(Fetched::NotModified))
    } else if status.is_redirection() {
        match response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
        {
            Some(location) => Ok(Hop::Redirect(location.to_owned())),
            None => Err(FetchError::Unavailable(format!(
                "{host} answered {status} without a location"
            ))),
        }
    } else if status.is_success() {
        let declared = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if declared.is_some_and(|n| n > max) {
            return Err(too_large(&host, max));
        }
        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let limit = usize::try_from(max).unwrap_or(usize::MAX);
        let body = Limited::new(response.into_body(), limit)
            .collect()
            .await
            .map_err(|e| {
                if e.downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some()
                {
                    too_large(&host, max)
                } else {
                    FetchError::Unavailable(format!("{host}: {e}"))
                }
            })?;
        Ok(Hop::Done(Fetched::Body {
            bytes: body.to_bytes().to_vec(),
            etag,
        }))
    } else {
        Err(FetchError::Unavailable(format!("{host} answered {status}")))
    };
    driver.abort();
    result
}

fn too_large(host: &str, max: u64) -> FetchError {
    FetchError::TooLarge(format!("{host} sent more than {} MiB", max >> 20))
}

fn parse(url: &str) -> Result<Uri, FetchError> {
    let uri: Uri = url
        .parse()
        .map_err(|_| FetchError::Refused(format!("{url} is not a URL")))?;
    if uri.host().is_none_or(str::is_empty) {
        return Err(FetchError::Refused(format!("{url} has no host")));
    }
    Ok(uri)
}

fn host_of(url: &str) -> String {
    url.parse::<Uri>()
        .ok()
        .and_then(|u| u.host().map(str::to_owned))
        .unwrap_or_else(|| url.to_owned())
}

/// Whether `host` is a loopback name or address.
pub fn is_loopback(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Checks that Kumikit may fetch `uri`, reached from `from` (a redirect) or directly.
fn check_scheme(uri: &Uri, from: Option<&Uri>) -> Result<(), FetchError> {
    let host = uri.host().unwrap_or_default();
    match uri.scheme_str() {
        Some("https") => Ok(()),
        Some("http") if is_loopback(host) => match from {
            None => Ok(()),
            Some(prev) if prev.scheme_str() == Some("http") => Ok(()),
            Some(_) => Err(FetchError::Refused(format!(
                "refusing a redirect from https to plain http ({host})"
            ))),
        },
        Some("http") => Err(FetchError::Refused(format!(
            "plain http is allowed only to this computer; use https for {host}"
        ))),
        other => Err(FetchError::Refused(format!(
            "cannot fetch from a {} URL",
            other.unwrap_or("relative")
        ))),
    }
}

/// Resolves a `Location` header against the URL that sent it.
fn resolve(base: &Uri, location: &str) -> Result<Uri, FetchError> {
    if location.contains("://") {
        return parse(location);
    }
    if let Some(path) = location.strip_prefix('/')
        && !path.starts_with('/')
    {
        let authority = base.authority().map(|a| a.as_str()).unwrap_or_default();
        let scheme = base.scheme_str().unwrap_or("https");
        return parse(&format!("{scheme}://{authority}/{path}"));
    }
    Err(FetchError::Refused(format!(
        "cannot follow the redirect to {location}"
    )))
}

/// The local path of a `file:` URL, if `url` is one.
pub fn file_path(url: &str) -> Option<std::path::PathBuf> {
    let rest = url.strip_prefix("file://")?;
    // `file:///C:/x` on Windows: drop the slash before the drive letter.
    let rest = match rest.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() && cfg!(windows) => &rest[1..],
        _ => rest,
    };
    Some(std::path::PathBuf::from(rest))
}

async fn read_file(path: std::path::PathBuf, max: u64) -> Result<Fetched, FetchError> {
    tokio::task::spawn_blocking(move || {
        let meta = std::fs::metadata(&path)
            .map_err(|e| FetchError::Unavailable(format!("cannot read {}: {e}", path.display())))?;
        if meta.len() > max {
            return Err(FetchError::TooLarge(format!(
                "{} is larger than {} MiB",
                path.display(),
                max >> 20
            )));
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| FetchError::Unavailable(format!("cannot read {}: {e}", path.display())))?;
        Ok(Fetched::Body { bytes, etag: None })
    })
    .await
    .map_err(|e| FetchError::Unavailable(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(s: &str) -> Uri {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_hosts() {
        for h in ["127.0.0.1", "localhost", "[::1]", "::1", "127.1.2.3"] {
            assert!(is_loopback(h), "{h}");
        }
        for h in ["example.com", "10.0.0.1", "localhost.example.com", "[::2]"] {
            assert!(!is_loopback(h), "{h}");
        }
    }

    #[test]
    fn scheme_rules() {
        assert!(check_scheme(&uri("https://example.com/i.json"), None).is_ok());
        assert!(check_scheme(&uri("http://127.0.0.1:9/i.json"), None).is_ok());
        assert!(check_scheme(&uri("http://example.com/i.json"), None).is_err());
        assert!(check_scheme(&uri("ftp://example.com/i.json"), None).is_err());
        // Redirects: never from https down to http, even to loopback.
        let https = uri("https://example.com/");
        assert!(check_scheme(&uri("http://127.0.0.1/x"), Some(&https)).is_err());
        let http = uri("http://127.0.0.1/");
        assert!(check_scheme(&uri("http://localhost:8/x"), Some(&http)).is_ok());
        assert!(check_scheme(&uri("https://example.com/x"), Some(&http)).is_ok());
    }

    #[test]
    fn redirect_resolution() {
        let base = uri("https://a.example:8443/x/y");
        assert_eq!(
            resolve(&base, "/z?q=1").unwrap(),
            uri("https://a.example:8443/z?q=1")
        );
        assert_eq!(
            resolve(&base, "https://b.example/w").unwrap(),
            uri("https://b.example/w")
        );
        assert!(resolve(&base, "//c.example/w").is_err());
        assert!(resolve(&base, "relative").is_err());
    }

    #[test]
    fn file_urls() {
        assert_eq!(
            file_path("file:///srv/index.json"),
            Some(std::path::PathBuf::from("/srv/index.json"))
        );
        assert_eq!(file_path("https://x/y"), None);
    }
}
