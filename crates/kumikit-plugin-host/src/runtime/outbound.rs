//! Outgoing HTTP for plugins (`docs/spec/04-plugin-host.md#outgoing-http`, ADR 0023).
//!
//! Guests use `wasi:http/outgoing-handler`; `wasmtime-wasi-http` hands every request to
//! [`Outbound::send_request`], which applies the plugin's `network` grant before anything
//! leaves the core: `https` only, the host (and a port other than 443) listed, every
//! resolved address public unless the entry is itself a private literal, no redirects
//! followed, limits per plugin. Logs name only the scheme and host of a request: paths
//! can hold tokens.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use tokio::sync::Semaphore;
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

/// Requests one plugin may have in flight.
pub const MAX_IN_FLIGHT: usize = 8;
/// Largest request body.
pub const MAX_REQUEST_BODY: usize = 8 << 20;
/// Largest response body.
pub const MAX_RESPONSE_BODY: usize = 16 << 20;
/// Time to connect (and finish TLS).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest a guest may wait for a response's first byte, or between two chunks.
pub const MAX_WAIT: Duration = Duration::from_secs(120);

/// Debug builds only: `host=http://127.0.0.1:port,…` sends a granted host's requests to a
/// loopback test server, after the policy accepted them.
pub const TEST_ROUTES_VAR: &str = "KUMIKIT_TEST_HTTP_ROUTES";

/// The sending side of one instance's `wasi:http`.
#[derive(Debug, Clone)]
pub struct Outbound {
    /// The plugin whose code runs, for logs and the in-flight limit.
    pub plugin: String,
    /// The `network` grant that applies (the contributing plugin's for a delegated session).
    pub network: BTreeSet<String>,
}

/// A request target the policy accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Host name or IP literal, as in the URL.
    pub host: String,
    /// Port (443 when the URL has none).
    pub port: u16,
    /// Whether the grant entry is a loopback or private IP literal, so private addresses
    /// are allowed.
    pub private_literal: bool,
}

/// Checks a URL's scheme, host and port against a `network` grant.
pub fn check_target(
    network: &BTreeSet<String>,
    scheme: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
) -> Result<Target, String> {
    if scheme != Some("https") {
        return Err(format!(
            "only https is allowed, not {}",
            scheme.unwrap_or("a URL without a scheme")
        ));
    }
    let host = host
        .filter(|h| !h.is_empty())
        .ok_or("the URL has no host")?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let port = port.unwrap_or(443);
    let wanted = if port == 443 {
        vec![host.clone(), format!("{host}:443")]
    } else {
        vec![format!("{host}:{port}")]
    };
    if !wanted.iter().any(|w| network.contains(w)) {
        return Err(if port == 443 {
            format!("{host} is not in permissions.network")
        } else {
            format!("{host}:{port} is not in permissions.network")
        });
    }
    let private_literal = host.parse::<IpAddr>().is_ok_and(|ip| !is_public(ip));
    Ok(Target {
        host,
        port,
        private_literal,
    })
}

/// Whether an address is on the public internet: not loopback, private, link-local,
/// shared (100.64/10), unspecified, multicast, broadcast, documentation or unique-local.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_multicast()
                || v.is_broadcast()
                || v.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || o[0] >= 240)
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v.segments();
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8))
        }
    }
}

/// Parses `host=http://127.0.0.1:port,…`; only loopback targets are taken.
pub fn parse_routes(text: &str) -> HashMap<String, SocketAddr> {
    text.split(',')
        .filter_map(|pair| {
            let (host, url) = pair.split_once('=')?;
            let addr: SocketAddr = url
                .trim()
                .strip_prefix("http://")?
                .trim_end_matches('/')
                .parse()
                .ok()?;
            addr.ip()
                .is_loopback()
                .then(|| (host.trim().to_ascii_lowercase(), addr))
        })
        .collect()
}

fn test_routes() -> &'static HashMap<String, SocketAddr> {
    static ROUTES: OnceLock<HashMap<String, SocketAddr>> = OnceLock::new();
    ROUTES.get_or_init(|| {
        let Ok(text) = std::env::var(TEST_ROUTES_VAR) else {
            return HashMap::new();
        };
        if cfg!(debug_assertions) {
            let routes = parse_routes(&text);
            tracing::warn!(hosts = ?routes.keys().collect::<Vec<_>>(), "test HTTP routes are on (debug build)");
            routes
        } else {
            tracing::warn!("{TEST_ROUTES_VAR} is ignored: this is a release build");
            HashMap::new()
        }
    })
}

fn in_flight(plugin: &str) -> Arc<Semaphore> {
    static LIMITS: OnceLock<Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();
    let map = LIMITS.get_or_init(Default::default);
    let mut map = map.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(plugin.to_owned())
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_IN_FLIGHT)))
        .clone()
}

fn tls() -> Result<Arc<rustls::ClientConfig>, String> {
    static TLS: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    TLS.get_or_init(|| {
        let found = rustls_native_certs::load_native_certs();
        let mut roots = rustls::RootCertStore::empty();
        let (added, _) = roots.add_parsable_certificates(found.certs);
        if added == 0 {
            return Err("no trusted certificates found on this system".into());
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Arc::new(config))
    })
    .clone()
}

type ResponseFuture = Box<
    dyn Future<
            Output = Result<
                (
                    http::Response<WasiBody>,
                    Box<dyn Future<Output = Result<(), Error>> + Send>,
                ),
                Error,
            >,
        > + Send,
>;

// `is_supported_scheme` keeps its default (http and https), so a plain `http` request
// reaches the policy below and is refused, logged, as `permission-denied`.
impl WasiHttpHooks for Outbound {
    fn send_request(
        &mut self,
        request: http::Request<WasiBody>,
        options: Option<RequestOptions>,
        _fut: Box<dyn Future<Output = Result<(), Error>> + Send>,
    ) -> ResponseFuture {
        let plugin = self.plugin.clone();
        let uri = request.uri();
        let checked = check_target(&self.network, uri.scheme_str(), uri.host(), uri.port_u16());
        Box::new(async move {
            let target = match checked {
                Ok(t) => t,
                Err(why) => {
                    tracing::warn!(%plugin, reason = %why, "permission denied: outgoing request");
                    return Err(Error::HttpRequestDenied);
                }
            };
            let permit = in_flight(&plugin)
                .try_acquire_owned()
                .map_err(|_| {
                    tracing::warn!(%plugin, "outgoing request refused: {MAX_IN_FLIGHT} already in flight");
                    Error::ConnectionLimitReached
                })?;
            tracing::debug!(%plugin, host = %target.host, port = target.port, "outgoing request");
            let r = send(&plugin, &target, request, options).await;
            drop(permit);
            r.map(|res| {
                (
                    res,
                    Box::new(async { Ok(()) }) as Box<dyn Future<Output = _> + Send>,
                )
            })
        })
    }
}

/// Connects to `target` (or its test route), checks the addresses, and sends.
async fn send(
    plugin: &str,
    target: &Target,
    mut request: http::Request<WasiBody>,
    options: Option<RequestOptions>,
) -> Result<http::Response<WasiBody>, Error> {
    let wait = |d: Option<Duration>| d.map_or(MAX_WAIT, |d| d.min(MAX_WAIT));
    let connect = options
        .and_then(|o| o.connect_timeout)
        .map_or(CONNECT_TIMEOUT, |d| d.min(CONNECT_TIMEOUT));
    let first_byte = wait(options.and_then(|o| o.first_byte_timeout));

    let route = test_routes().get(&target.host).copied();
    let tcp = match route {
        Some(addr) => tokio::time::timeout(connect, tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| Error::ConnectionTimeout)?
            .map_err(Error::Connect)?,
        None => {
            let addrs: Vec<SocketAddr> = tokio::time::timeout(
                connect,
                tokio::net::lookup_host((target.host.as_str(), target.port)),
            )
            .await
            .map_err(|_| Error::DnsTimeout)?
            .map_err(|_| Error::DestinationNotFound)?
            .collect();
            if addrs.is_empty() {
                return Err(Error::DestinationNotFound);
            }
            if !target.private_literal && addrs.iter().any(|a| !is_public(a.ip())) {
                tracing::warn!(%plugin, host = %target.host, "permission denied: outgoing request to a private address");
                return Err(Error::DestinationIpProhibited);
            }
            tokio::time::timeout(connect, tokio::net::TcpStream::connect(&addrs[..]))
                .await
                .map_err(|_| Error::ConnectionTimeout)?
                .map_err(Error::Connect)?
        }
    };

    // Only the path goes on the wire (no proxy); the body is capped.
    let path = request
        .uri()
        .path_and_query()
        .map_or("/", |p| p.as_str())
        .to_owned();
    *request.uri_mut() = path.parse().map_err(|_| Error::HttpRequestUriInvalid)?;
    let request = request.map(|body| {
        Limited::new(body, MAX_REQUEST_BODY)
            .map_err(|_| Error::HttpRequestBodySize(Some(MAX_REQUEST_BODY as u64)))
            .boxed_unsync()
    });

    let response = if route.is_some() {
        exchange(tcp, request, connect, first_byte).await?
    } else {
        let config = tls().map_err(|e| {
            tracing::warn!(%plugin, error = %e, "outgoing request: no TLS configuration");
            Error::TlsCertificateError
        })?;
        let name = rustls::pki_types::ServerName::try_from(target.host.clone())
            .map_err(|_| Error::HttpRequestUriInvalid)?;
        let stream = tokio::time::timeout(
            connect,
            tokio_rustls::TlsConnector::from(config).connect(name, tcp),
        )
        .await
        .map_err(|_| Error::ConnectionTimeout)?
        .map_err(Error::Tls)?;
        exchange(stream, request, connect, first_byte).await?
    };
    Ok(response.map(|body| {
        Limited::new(body, MAX_RESPONSE_BODY)
            .map_err(|e| match e.downcast::<hyper::Error>() {
                Ok(e) => Error::from(*e),
                Err(_) => Error::HttpResponseBodySize(Some(MAX_RESPONSE_BODY as u64)),
            })
            .boxed_unsync()
    }))
}

/// One HTTP/1.1 exchange on `stream`; the connection is driven by its own task.
async fn exchange<S>(
    stream: S,
    request: http::Request<http_body_util::combinators::UnsyncBoxBody<Bytes, Error>>,
    connect: Duration,
    first_byte: Duration,
) -> Result<http::Response<hyper::body::Incoming>, Error>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let (mut sender, conn) = tokio::time::timeout(
        connect,
        hyper::client::conn::http1::handshake(wasmtime_wasi_http::io::TokioIo::new(stream)),
    )
    .await
    .map_err(|_| Error::ConnectionTimeout)??;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::debug!(error = %e, "outgoing connection ended");
        }
    });
    tokio::time::timeout(first_byte, sender.send_request(request))
        .await
        .map_err(|_| Error::ConnectionReadTimeout)?
        .map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(hosts: &[&str]) -> BTreeSet<String> {
        hosts.iter().map(|h| (*h).to_owned()).collect()
    }

    #[test]
    fn only_https_to_listed_hosts_and_ports() {
        let g = grant(&["api.telegram.org", "git.example:8443", "10.0.0.5"]);
        let ok = |s, h, p| check_target(&g, Some(s), Some(h), p);
        assert_eq!(ok("https", "api.telegram.org", None).unwrap().port, 443);
        assert!(ok("https", "API.Telegram.org", Some(443)).is_ok());
        assert!(
            ok("http", "api.telegram.org", None)
                .unwrap_err()
                .contains("only https")
        );
        assert!(
            ok("https", "api.telegram.org", Some(8443))
                .unwrap_err()
                .contains(":8443")
        );
        assert!(
            ok("https", "evil.example", None)
                .unwrap_err()
                .contains("not in permissions.network")
        );
        assert!(ok("https", "git.example", Some(8443)).is_ok());
        assert!(ok("https", "git.example", None).is_err());
        assert!(ok("https", "10.0.0.5", None).unwrap().private_literal);
        assert!(
            !ok("https", "api.telegram.org", None)
                .unwrap()
                .private_literal
        );
        assert!(check_target(&g, None, Some("api.telegram.org"), None).is_err());
    }

    #[test]
    fn private_and_special_addresses_are_not_public() {
        for a in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
        ] {
            assert!(!is_public(a.parse().unwrap()), "{a}");
        }
        for a in ["149.154.167.220", "140.82.112.6", "2606:4700::1111"] {
            assert!(is_public(a.parse().unwrap()), "{a}");
        }
    }

    #[test]
    fn test_routes_take_only_loopback_targets() {
        let r = parse_routes(
            "api.telegram.org=http://127.0.0.1:4100, Api.GitHub.com=http://127.0.0.1:4200/,x=http://10.0.0.1:80,bad",
        );
        assert_eq!(r.len(), 2);
        assert_eq!(r["api.github.com"].port(), 4200);
    }
}
