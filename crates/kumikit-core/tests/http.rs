//! Outgoing HTTP for plugins against the real binary (`docs/spec/04-plugin-host.md#outgoing-http`):
//! a granted host is reached (through the debug-only test route), and every other
//! request is refused with `permission_denied` before anything is sent.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use support::{Client, Core, DEADLINE, Dirs, PROBE_MANIFEST, install, kkp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

/// A plain-HTTP server on loopback that answers every request with `pong <path>` and
/// counts requests and keeps the last request head.
async fn server() -> (u16, Arc<AtomicUsize>, Arc<std::sync::Mutex<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let last = Arc::new(std::sync::Mutex::new(String::new()));
    let (h, l) = (hits.clone(), last.clone());
    tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            let (h, l) = (h.clone(), l.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut got = Vec::new();
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = s.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    got.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&got).into_owned();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
                h.fetch_add(1, Ordering::SeqCst);
                *l.lock().unwrap() = head;
                let body = format!("pong {path}");
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                s.write_all(reply.as_bytes()).await.unwrap();
            });
        }
    });
    (port, hits, last)
}

async fn turn(c: &mut Client, thread: &str, text: &str) -> Vec<Value> {
    c.call(
        "thread.send",
        json!({ "thread": thread, "input": { "text": text } }),
    )
    .await;
    c.events_until("turn.ended").await
}

#[tokio::test(flavor = "multi_thread")]
async fn only_granted_https_hosts_are_reached() {
    within(async {
        let (port, hits, last) = server().await;
        let dirs = Dirs::new();
        let route = format!("api.example.test=http://127.0.0.1:{port}");
        let (_core, _) = Core::start_with_env(&dirs, None, &[], &[("KUMIKIT_TEST_HTTP_ROUTES", &route)]).await;
        let mut c = Client::hello(&dirs.address).await;
        let manifest = PROBE_MANIFEST.replace(
            "spawn = [\"fake-acp-agent\"]",
            "spawn = [\"fake-acp-agent\"]\nnetwork = [\"api.example.test\", \"localhost\"]",
        );
        let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
        install(&mut c, &kkp(&dirs, "probe-http", &manifest, &[("plugin.wasm", &wasm)])).await;
        let ws = c.call("workspace.open", json!({ "path": dirs.workspace() })).await;
        let thread = c
            .call(
                "thread.create",
                json!({ "workspace": ws["workspace"]["id"], "provider": "test.probe/probe", "model": "probe" }),
            )
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        c.call("events.subscribe", json!({ "topic": format!("thread:{thread}"), "after": 0 }))
            .await;

        let events = turn(&mut c, &thread, "http https://api.example.test/bot123:token/getMe?x=1").await;
        let done = events.iter().find(|e| e["kind"] == "text.done").expect("a reply");
        assert_eq!(done["data"]["text"], "200 pong /bot123:token/getMe?x=1");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(last.lock().unwrap().to_ascii_lowercase().contains("host: api.example.test"));

        for (url, why) in [
            ("http://api.example.test/", "plain http"),
            ("https://evil.example/", "a host not granted"),
            ("https://api.example.test:8443/", "a port not granted"),
            ("https://localhost/", "a granted name that resolves to loopback"),
        ] {
            let events = turn(&mut c, &thread, &format!("http {url}")).await;
            let ended = &events.last().unwrap()["data"];
            assert_eq!(ended["outcome"], "failed", "{why}: {ended}");
            assert!(
                ended["error"].as_str().unwrap().starts_with("permission_denied: "),
                "{why}: {ended}"
            );
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1, "a refused request reached the server");
    })
    .await;
}
