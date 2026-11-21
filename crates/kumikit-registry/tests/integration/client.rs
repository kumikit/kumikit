//! The registry client's cache and download checks, over loopback.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use kumikit_registry::client::{Freshness, MAX_AGE, RegistryClient, RegistryError};
use kumikit_registry::fetch::{Fetcher, Limits};
use kumikit_registry::index::{FORMAT, Index, Version};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A tiny server: `/index.json` with an ETag (304 when it matches), `/p.kkp` bytes.
struct Server {
    url: String,
    hits: Arc<AtomicUsize>,
    index: Arc<Mutex<(String, String)>>, // (body, etag)
    down: Arc<std::sync::atomic::AtomicBool>,
}

async fn server(package: Vec<u8>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let index = Arc::new(Mutex::new((String::new(), "\"1\"".to_owned())));
    let down = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (h, i, d) = (hits.clone(), index.clone(), down.clone());
    tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            let (h, i, d, package) = (h.clone(), i.clone(), d.clone(), package.clone());
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if s.read(&mut b).await.unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(b[0]);
                }
                h.fetch_add(1, Ordering::SeqCst);
                let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
                let reply = if d.load(Ordering::SeqCst) {
                    b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_vec()
                } else if head.starts_with("get /index.json") {
                    let (body, etag) = i.lock().unwrap().clone();
                    if head.contains(&format!("if-none-match: {}", etag.to_ascii_lowercase())) {
                        b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".to_vec()
                    } else {
                        format!(
                            "HTTP/1.1 200 OK\r\nETag: {etag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .into_bytes()
                    }
                } else {
                    let mut r = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        package.len()
                    )
                    .into_bytes();
                    r.extend_from_slice(&package);
                    r
                };
                let _ = s.write_all(&reply).await;
                let _ = s.shutdown().await;
            });
        }
    });
    Server {
        url,
        hits,
        index,
        down,
    }
}

fn index_json(name: &str) -> String {
    serde_json::to_string(&Index {
        format: FORMAT,
        name: name.into(),
        generated: "2026-11-18T00:00:00Z".into(),
        starter: vec![],
        publishers: BTreeMap::new(),
        plugins: vec![],
    })
    .unwrap()
}

/// A clock the test moves by hand.
fn clock() -> (Arc<AtomicU64>, Arc<dyn Fn() -> SystemTime + Send + Sync>) {
    let secs = Arc::new(AtomicU64::new(1_800_000_000));
    let s = secs.clone();
    (
        secs,
        Arc::new(move || SystemTime::UNIX_EPOCH + Duration::from_secs(s.load(Ordering::SeqCst))),
    )
}

#[tokio::test]
async fn the_cache_is_reused_then_revalidated_with_its_etag() {
    let srv = server(vec![]).await;
    *srv.index.lock().unwrap() = (index_json("one"), "\"1\"".into());
    let dir = tempfile::tempdir().unwrap();
    let (secs, now) = clock();
    let client =
        RegistryClient::new(Fetcher::new(Limits::default()), dir.path().to_owned()).with_clock(now);
    let url = format!("{}/index.json", srv.url);

    let first = client.index(&url, Freshness::Cached).await.unwrap();
    assert_eq!(first.index.name, "one");
    assert_eq!(srv.hits.load(Ordering::SeqCst), 1);

    // Fresh: browsing does not ask again.
    client.index(&url, Freshness::Cached).await.unwrap();
    assert_eq!(srv.hits.load(Ordering::SeqCst), 1);

    // Installing revalidates; the server says nothing changed.
    let again = client.index(&url, Freshness::Revalidate).await.unwrap();
    assert_eq!(again.index.name, "one");
    assert_eq!(srv.hits.load(Ordering::SeqCst), 2);

    // Older than MAX_AGE: browsing asks again and gets the new index.
    *srv.index.lock().unwrap() = (index_json("two"), "\"2\"".into());
    secs.fetch_add(MAX_AGE.as_secs() + 1, Ordering::SeqCst);
    let newer = client.index(&url, Freshness::Cached).await.unwrap();
    assert_eq!(newer.index.name, "two");
    assert_eq!(srv.hits.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn an_unreachable_registry_falls_back_to_the_cache_only_for_browsing() {
    let srv = server(vec![]).await;
    *srv.index.lock().unwrap() = (index_json("one"), "\"1\"".into());
    let dir = tempfile::tempdir().unwrap();
    let (secs, now) = clock();
    let client =
        RegistryClient::new(Fetcher::new(Limits::default()), dir.path().to_owned()).with_clock(now);
    let url = format!("{}/index.json", srv.url);
    client.index(&url, Freshness::Cached).await.unwrap();
    secs.fetch_add(MAX_AGE.as_secs() + 1, Ordering::SeqCst);
    srv.down.store(true, Ordering::SeqCst);
    assert_eq!(
        client
            .index(&url, Freshness::Cached)
            .await
            .unwrap()
            .index
            .name,
        "one"
    );
    let err = client.index(&url, Freshness::Revalidate).await.unwrap_err();
    assert!(matches!(err, RegistryError::Unavailable(_)), "{err}");
}

#[tokio::test]
async fn downloads_must_match_the_index_and_the_lock() {
    let package = b"pretend package".to_vec();
    let srv = server(package.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let client = RegistryClient::new(Fetcher::new(Limits::default()), dir.path().to_owned());
    let hash: String = Sha256::digest(&package)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut v = Version {
        version: "0.1.0".into(),
        url: format!("{}/p.kkp", srv.url),
        sha256: hash.clone(),
        api: "^0.1".into(),
        requested: None,
        ui: None,
        yanked: false,
    };
    assert_eq!(client.download(&v, None).await.unwrap(), package);
    assert_eq!(client.download(&v, Some(&hash)).await.unwrap(), package);
    let err = client
        .download(&v, Some(&"0".repeat(64)))
        .await
        .unwrap_err();
    assert!(
        matches!(err, RegistryError::Untrusted(ref m) if m.contains("lockfile")),
        "{err}"
    );
    v.sha256 = "f".repeat(64);
    let err = client.download(&v, None).await.unwrap_err();
    assert!(
        matches!(err, RegistryError::Untrusted(ref m) if m.contains("SHA-256")),
        "{err}"
    );
}

#[tokio::test]
async fn refused_urls_make_no_request() {
    let dir = tempfile::tempdir().unwrap();
    let client = RegistryClient::new(Fetcher::new(Limits::default()), dir.path().to_owned());
    let err = client
        .index("http://example.com/index.json", Freshness::Cached)
        .await
        .unwrap_err();
    assert!(matches!(err, RegistryError::Invalid(_)), "{err}");
}
