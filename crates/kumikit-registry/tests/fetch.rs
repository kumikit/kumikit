//! Fetching over loopback: plain HTTP, HTTPS with a test certificate authority, redirects,
//! caching headers, limits and timeouts. Nothing leaves this machine.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kumikit_registry::fetch::{FetchError, Fetched, Fetcher, Limits};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

type Route = Arc<dyn Fn(&str, &str) -> Vec<u8> + Send + Sync>;

/// Answers each request with `route(path, request head)`; counts requests.
async fn serve(route: Route, tls: Option<tokio_rustls::TlsAcceptor>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let route = route.clone();
            let seen = seen.clone();
            let tls = tls.clone();
            tokio::spawn(async move {
                match tls {
                    Some(acceptor) => {
                        if let Ok(s) = acceptor.accept(tcp).await {
                            answer(s, route, seen).await;
                        }
                    }
                    None => answer(tcp, route, seen).await,
                }
            });
        }
    });
    (format!("127.0.0.1:{}", addr.port()), count)
}

async fn answer<S: AsyncRead + AsyncWrite + Unpin>(mut s: S, route: Route, seen: Arc<AtomicUsize>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if s.read(&mut byte).await.unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    seen.fetch_add(1, Ordering::SeqCst);
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let response = route(&path, &head);
    if response.is_empty() {
        // Never answer: hold the connection open.
        std::future::pending::<()>().await;
    }
    let _ = s.write_all(&response).await;
    let _ = s.shutdown().await;
}

fn ok(body: &str, extra: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn redirect(to: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}

fn fetcher() -> Fetcher {
    Fetcher::new(Limits::default())
}

const MIB: u64 = 1 << 20;

#[tokio::test]
async fn plain_http_on_loopback() {
    let (addr, count) = serve(
        Arc::new(|_, _| ok("{\"format\":1}", "ETag: \"v1\"\r\n")),
        None,
    )
    .await;
    let got = fetcher()
        .get(&format!("http://{addr}/index.json"), None, MIB)
        .await
        .unwrap();
    assert_eq!(
        got,
        Fetched::Body {
            bytes: b"{\"format\":1}".to_vec(),
            etag: Some("\"v1\"".into())
        }
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn etag_gives_not_modified() {
    let (addr, _) = serve(
        Arc::new(|_, head| {
            if head.to_ascii_lowercase().contains("if-none-match: \"v1\"") {
                b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".to_vec()
            } else {
                ok("{}", "ETag: \"v1\"\r\n")
            }
        }),
        None,
    )
    .await;
    let url = format!("http://{addr}/index.json");
    assert_eq!(
        fetcher().get(&url, Some("\"v1\""), MIB).await.unwrap(),
        Fetched::NotModified
    );
    assert!(matches!(
        fetcher().get(&url, Some("\"v0\""), MIB).await.unwrap(),
        Fetched::Body { .. }
    ));
}

#[tokio::test]
async fn follows_redirects_and_stops_loops() {
    let (addr, _) = serve(
        Arc::new(|path, _| match path {
            "/a" => redirect("/b"),
            "/b" => ok("done", ""),
            _ => redirect("/loop"),
        }),
        None,
    )
    .await;
    let got = fetcher()
        .get(&format!("http://{addr}/a"), None, MIB)
        .await
        .unwrap();
    assert!(matches!(got, Fetched::Body { ref bytes, .. } if bytes == b"done"));
    let err = fetcher()
        .get(&format!("http://{addr}/loop"), None, MIB)
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::Unavailable(ref m) if m.contains("redirects")),
        "{err}"
    );
}

#[tokio::test]
async fn redirect_off_loopback_to_plain_http_is_refused() {
    let (addr, _) = serve(Arc::new(|_, _| redirect("http://example.com/x")), None).await;
    let err = fetcher()
        .get(&format!("http://{addr}/a"), None, MIB)
        .await
        .unwrap_err();
    assert!(matches!(err, FetchError::Refused(_)), "{err}");
}

#[tokio::test]
async fn plain_http_off_loopback_is_refused_without_connecting() {
    let err = fetcher()
        .get("http://example.com/index.json", None, MIB)
        .await
        .unwrap_err();
    assert!(matches!(err, FetchError::Refused(_)), "{err}");
}

#[tokio::test]
async fn size_cap_declared_and_streamed() {
    let big = "x".repeat(2048);
    let declared = ok(&big, "");
    let (addr, _) = serve(
        Arc::new(move |path, _| {
            if path == "/declared" {
                declared.clone()
            } else {
                // No Content-Length: the body is cut off by the stream limit.
                format!(
                    "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{}",
                    "y".repeat(4096)
                )
                .into_bytes()
            }
        }),
        None,
    )
    .await;
    for path in ["declared", "streamed"] {
        let err = fetcher()
            .get(&format!("http://{addr}/{path}"), None, 1024)
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::TooLarge(_)), "{path}: {err}");
    }
}

#[tokio::test]
async fn server_errors_are_unavailable() {
    let (addr, _) = serve(
        Arc::new(|_, _| {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }),
        None,
    )
    .await;
    let err = fetcher()
        .get(&format!("http://{addr}/missing"), None, MIB)
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::Unavailable(ref m) if m.contains("404")),
        "{err}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_silent_server_times_out() {
    let (addr, _) = serve(Arc::new(|_, _| Vec::new()), None).await;
    let limits = Limits {
        connect: Duration::from_secs(10),
        request: Duration::from_secs(30),
    };
    let err = Fetcher::new(limits)
        .get(&format!("http://{addr}/slow"), None, MIB)
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::Unavailable(ref m) if m.contains("did not answer")),
        "{err}"
    );
}

#[tokio::test]
async fn https_with_a_test_root() {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            rustls::pki_types::PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap(),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let (addr, _) = serve(Arc::new(|_, _| ok("secure", "")), Some(acceptor)).await;
    let port = addr.rsplit(':').next().unwrap();

    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    let trusted = Fetcher::with_roots(roots, Limits::default());
    let got = trusted
        .get(&format!("https://localhost:{port}/i"), None, MIB)
        .await
        .unwrap();
    assert!(matches!(got, Fetched::Body { ref bytes, .. } if bytes == b"secure"));

    // A certificate from an authority the fetcher does not trust is refused.
    let untrusted = Fetcher::with_roots(rustls::RootCertStore::empty(), Limits::default());
    let err = untrusted
        .get(&format!("https://localhost:{port}/i"), None, MIB)
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::Unavailable(ref m) if m.contains("TLS")),
        "{err}"
    );
}

#[tokio::test]
async fn file_urls_and_their_cap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.json");
    std::fs::write(&path, "{}").unwrap();
    let url = format!("file://{}", path.to_string_lossy().replace('\\', "/"));
    let url = if cfg!(windows) {
        url.replacen("file://", "file:///", 1)
    } else {
        url
    };
    let got = fetcher().get(&url, None, MIB).await.unwrap();
    assert_eq!(
        got,
        Fetched::Body {
            bytes: b"{}".to_vec(),
            etag: None
        }
    );
    assert!(matches!(
        fetcher().get(&url, None, 1).await.unwrap_err(),
        FetchError::TooLarge(_)
    ));
}

#[tokio::test]
async fn system_roots_load() {
    // The OS store is read on the first https fetch; a closed port fails after loading it,
    // so this checks the store loads without touching the network.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let err = fetcher()
        .get(&format!("https://localhost:{port}/x"), None, MIB)
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::Unavailable(ref m) if m.contains("cannot reach")),
        "{err}"
    );
}
