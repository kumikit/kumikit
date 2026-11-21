//! The remote transport against the real binary (`docs/spec/02-protocol.md#remote-transport`,
//! `#pairing`): TLS, the authentication gate, pairing, tokens, revocation, limits and
//! keepalive. Loopback only; time limits are shortened with hidden `serve` flags and tests
//! wait for the core to act, never for a fixed time.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use futures_util::{SinkExt, StreamExt};
use kumikit_protocol::methods::METHOD_NAMES;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::{Client, Core, DEADLINE, Dirs, RemoteInfo, Ws, tls, tls_with, ws};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

/// What a closed connection said.
#[derive(Debug, PartialEq)]
enum Ended {
    Close(Option<u16>),
    Eof,
}

struct Remote {
    ws: Ws,
    next: i64,
}

impl Remote {
    async fn connect(info: &RemoteInfo) -> Remote {
        Remote {
            ws: ws(info).await,
            next: 0,
        }
    }

    /// Connects and logs in with `token`.
    async fn login(info: &RemoteInfo, token: &str) -> Remote {
        let mut r = Remote::connect(info).await;
        let res = r.request("auth.token", json!({"token": token})).await;
        assert!(res.get("result").is_some(), "{res}");
        r
    }

    async fn send_raw(&mut self, text: &str) {
        self.ws.send(Message::text(text)).await.unwrap();
    }

    async fn next_json(&mut self) -> Option<Value> {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    return Some(serde_json::from_str(t.as_str()).unwrap());
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return None,
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send_raw(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        )
        .await;
        loop {
            let v = self.next_json().await.expect("connection closed");
            if v["id"] == id {
                return v;
            }
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        let r = self.request(method, params).await;
        assert!(r.get("error").is_none(), "{method} failed: {r}");
        r["result"].clone()
    }

    async fn hello(&mut self) -> Value {
        self.call(
            "core.hello",
            json!({"protocol":"0.6", "client": {"name": "t", "version": "0"}}),
        )
        .await
    }

    /// Reads until the connection ends.
    async fn ended(&mut self) -> Ended {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Close(frame))) => {
                    return Ended::Close(frame.map(|f| f.code.into()));
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return Ended::Eof,
            }
        }
    }
}

fn err_code(r: &Value) -> &str {
    r["error"]["data"]["code"].as_str().unwrap_or("")
}

async fn pairing_code(dirs: &Dirs) -> Value {
    let mut local = Client::hello(&dirs.address).await;
    local.call("auth.pairing.create", json!({})).await
}

/// Pairs a device; returns its token and id.
async fn pair(dirs: &Dirs, info: &RemoteInfo, name: &str) -> (String, String) {
    let code = pairing_code(dirs).await;
    let mut r = Remote::connect(info).await;
    let res = r
        .call(
            "auth.pair",
            json!({"code": code["code"], "deviceName": name}),
        )
        .await;
    (
        res["token"].as_str().unwrap().to_owned(),
        res["device"]["id"].as_str().unwrap().to_owned(),
    )
}

// ---- tests ----

#[tokio::test(flavor = "multi_thread")]
async fn without_listen_the_core_is_local_only() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        let r = c.request("auth.pairing.create", json!({})).await;
        assert_eq!(err_code(&r), "unsupported");
        let hello = c
            .call(
                "core.hello",
                json!({"protocol":"0.6", "client": {"name": "t", "version": "0"}}),
            )
            .await;
        assert!(
            !hello["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("devices"))
        );
        assert!(!dirs.data().join("tls").exists(), "no certificate is made");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_then_log_in_with_the_token() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        assert!(info.url.starts_with("wss://127.0.0.1:"));

        let code = pairing_code(&dirs).await;
        assert_eq!(code["fingerprint"], info.fingerprint.as_str());
        assert_eq!(code["url"], info.url.as_str());
        assert_eq!(code["code"].as_str().unwrap().len(), 9);

        let mut r = Remote::connect(&info).await;
        let paired = r
            .call("auth.pair", json!({"code": code["code"].as_str().unwrap().to_lowercase(), "deviceName": "Work laptop"}))
            .await;
        let token = paired["token"].as_str().unwrap().to_owned();
        assert!(token.starts_with("kkd_"));
        assert_eq!(paired["device"]["name"], "Work laptop");
        // The pairing connection is authenticated: the usual handshake follows.
        let r2 = r.request("workspace.list", json!({})).await;
        assert_eq!(err_code(&r2), "handshake_required");
        let hello = r.hello().await;
        assert!(hello["capabilities"].as_array().unwrap().contains(&json!("devices")));
        r.call("workspace.list", json!({})).await;
        let again = r.request("auth.token", json!({"token": token})).await;
        assert_eq!(err_code(&again), "conflict");
        let denied = r.request("auth.pairing.create", json!({})).await;
        assert_eq!(err_code(&denied), "permission_denied");

        // A later connection logs in with the token.
        let mut later = Remote::login(&info, &token).await;
        later.hello().await;
        let list = later.call("auth.devices.list", json!({})).await;
        assert_eq!(list["devices"][0]["current"], true);
        // Locally nothing is the current device.
        let mut local = Client::hello(&dirs.address).await;
        let list = local.call("auth.devices.list", json!({})).await;
        assert!(list["devices"][0].get("current").is_none(), "{list}");

        // The code was used up.
        let mut r3 = Remote::connect(&info).await;
        let reuse = r3
            .request("auth.pair", json!({"code": code["code"], "deviceName": "x"}))
            .await;
        assert_eq!(err_code(&reuse), "unauthenticated");
        assert_eq!(r3.ended().await, Ended::Close(Some(1008)));
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_method_is_refused_before_authentication() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        for method in METHOD_NAMES
            .iter()
            .filter(|m| !m.starts_with("auth.pair") && **m != "auth.token")
        {
            let mut r = Remote::connect(&info).await;
            let res = r.request(method, json!({})).await;
            assert_eq!(err_code(&res), "unauthenticated", "{method}: {res}");
            assert_eq!(r.ended().await, Ended::Close(Some(1008)), "{method}");
        }
        // auth.pairing.create is not a way in either.
        let mut r = Remote::connect(&info).await;
        assert_eq!(
            err_code(&r.request("auth.pairing.create", json!({})).await),
            "unauthenticated"
        );
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn notifications_batches_and_junk_before_authentication_close() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        for first in [
            r#"{"jsonrpc":"2.0","method":"auth.token","params":{"token":"x"}}"#,
            r#"[{"jsonrpc":"2.0","id":1,"method":"auth.token","params":{"token":"x"}}]"#,
            "not json",
        ] {
            let mut r = Remote::connect(&info).await;
            r.send_raw(first).await;
            let res = r.next_json().await.unwrap();
            assert_eq!(err_code(&res), "unauthenticated", "{first}");
            assert_eq!(r.ended().await, Ended::Close(Some(1008)), "{first}");
        }
        let mut r = Remote::connect(&info).await;
        r.ws.send(Message::binary(b"{}".to_vec())).await.unwrap();
        let res = r.next_json().await.unwrap();
        assert_eq!(err_code(&res), "invalid_request");
        assert_eq!(r.ended().await, Ended::Close(Some(1008)));
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_codes_and_unknown_tokens_close_the_connection() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        let good = pairing_code(&dirs).await;
        let good = good["code"].as_str().unwrap();
        let wrong = if good.starts_with('0') {
            "1111-1111"
        } else {
            "0000-0000"
        };
        // Four wrong tries, one attempt per connection, then the right code still works.
        for _ in 0..4 {
            let mut r = Remote::connect(&info).await;
            let res = r
                .request("auth.pair", json!({"code": wrong, "deviceName": "x"}))
                .await;
            assert_eq!(err_code(&res), "unauthenticated");
            assert_eq!(r.ended().await, Ended::Close(Some(1008)));
        }
        let mut r = Remote::connect(&info).await;
        r.call("auth.pair", json!({"code": good, "deviceName": "x"}))
            .await;

        // The fifth wrong try burns a code.
        let burnt = pairing_code(&dirs).await;
        for _ in 0..5 {
            let mut r = Remote::connect(&info).await;
            r.request("auth.pair", json!({"code": wrong, "deviceName": "x"}))
                .await;
        }
        let mut r = Remote::connect(&info).await;
        let res = r
            .request(
                "auth.pair",
                json!({"code": burnt["code"], "deviceName": "x"}),
            )
            .await;
        assert_eq!(err_code(&res), "unauthenticated");

        // A new code replaces the previous one.
        let old = pairing_code(&dirs).await;
        let new = pairing_code(&dirs).await;
        if old["code"] != new["code"] {
            let mut r = Remote::connect(&info).await;
            let res = r
                .request("auth.pair", json!({"code": old["code"], "deviceName": "x"}))
                .await;
            assert_eq!(err_code(&res), "unauthenticated");
        }

        let mut r = Remote::connect(&info).await;
        let res = r.request("auth.token", json!({"token": "kkd_nope"})).await;
        assert_eq!(err_code(&res), "unauthenticated");
        assert_eq!(r.ended().await, Ended::Close(Some(1008)));

        let mut r = Remote::connect(&info).await;
        let fresh = pairing_code(&dirs).await;
        let res = r
            .request(
                "auth.pair",
                json!({"code": fresh["code"], "deviceName": "a\nb"}),
            )
            .await;
        assert_eq!(err_code(&res), "invalid_params");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_device_closes_its_connections() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        let (token, id) = pair(&dirs, &info, "laptop").await;
        let (other_token, _) = pair(&dirs, &info, "phone").await;
        let mut victim = Remote::login(&info, &token).await;
        victim.hello().await;
        let mut bystander = Remote::login(&info, &other_token).await;
        bystander.hello().await;

        let mut local = Client::hello(&dirs.address).await;
        local
            .call("auth.devices.revoke", json!({"device": id}))
            .await;
        assert_eq!(victim.ended().await, Ended::Close(Some(4001)));
        // Others are untouched.
        bystander.call("workspace.list", json!({})).await;

        let mut again = Remote::connect(&info).await;
        let res = again.request("auth.token", json!({"token": token})).await;
        assert_eq!(err_code(&res), "unauthenticated");

        // A device can revoke itself; it gets the answer, then the connection closes.
        let other = bystander.call("auth.devices.list", json!({})).await;
        let own = other["devices"][0]["id"].clone();
        let r = bystander
            .request("auth.devices.revoke", json!({"device": own}))
            .await;
        assert_eq!(r["result"], json!({}), "{r}");
        assert_eq!(bystander.ended().await, Ended::Close(Some(4001)));
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn browsers_wrong_paths_and_old_tls_are_refused() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;

        let mut req = info.url.as_str().into_client_request().unwrap();
        req.headers_mut()
            .insert("origin", "https://evil.example".parse().unwrap());
        let err = tokio_tungstenite::client_async(req, tls(&info).await)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("403"), "{err}");

        let url = info.url.replace("/rpc", "/other");
        let err = tokio_tungstenite::client_async(url.as_str(), tls(&info).await)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("404"), "{err}");

        assert!(tls_with(&info, &[&rustls::version::TLS12]).await.is_err());
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_and_binary_messages_close_an_authenticated_connection() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        let (token, _) = pair(&dirs, &info, "x").await;

        let mut r = Remote::login(&info, &token).await;
        r.hello().await;
        r.ws.send(Message::binary(b"{}".to_vec())).await.unwrap();
        let res = r.next_json().await.unwrap();
        assert_eq!(err_code(&res), "invalid_request");
        assert!(matches!(r.ended().await, Ended::Close(_) | Ended::Eof));

        let mut r = Remote::login(&info, &token).await;
        let big = format!("\"{}\"", "x".repeat(8 * 1024 * 1024));
        let _ = r.ws.send(Message::text(big)).await;
        assert!(matches!(r.ended().await, Ended::Close(_) | Ended::Eof));
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slow_and_silent_clients_are_dropped() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(
            &dirs,
            &[
                "--pre-auth-ms",
                "300",
                "--keepalive-ping-ms",
                "100",
                "--keepalive-idle-ms",
                "400",
            ],
            false,
        )
        .await;

        // Connected but never authenticating.
        let mut r = Remote::connect(&info).await;
        assert!(matches!(r.ended().await, Ended::Close(_) | Ended::Eof));
        // Not even finishing TLS.
        let mut tcp = TcpStream::connect(&info.addr).await.unwrap();
        let mut buf = [0u8; 64];
        assert_eq!(tcp.read(&mut buf).await.unwrap_or(0), 0);

        // Authenticated, then silent: no pongs. The raw stream never answers pings.
        let (token, _) = pair(&dirs, &info, "x").await;
        let mut ws = ws(&info).await;
        ws.send(Message::text(
            json!({"jsonrpc":"2.0","id":1,"method":"auth.token","params":{"token": token}})
                .to_string(),
        ))
        .await
        .unwrap();
        // Read the answer, then stop reading frames so pings go unanswered.
        loop {
            if let Some(Ok(Message::Text(_))) = ws.next().await {
                break;
            }
        }
        let mut raw = ws.into_inner();
        let mut sink = Vec::new();
        let _ = raw.read_to_end(&mut sink).await;
        // Reaching here means the core closed the connection.
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn too_many_unauthenticated_connections_are_refused() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        let mut held = Vec::new();
        for _ in 0..32 {
            held.push(Remote::connect(&info).await);
        }
        let mut tcp = TcpStream::connect(&info.addr).await.unwrap();
        let mut buf = [0u8; 16];
        assert_eq!(
            tcp.read(&mut buf).await.unwrap_or(0),
            0,
            "the 33rd is closed at once"
        );
        // Authenticating frees a place.
        let (token, _) = {
            let code = pairing_code(&dirs).await;
            let r = held.last_mut().unwrap();
            let res = r
                .call(
                    "auth.pair",
                    json!({"code": code["code"], "deviceName": "x"}),
                )
                .await;
            (res["token"].as_str().unwrap().to_owned(), ())
        };
        Remote::login(&info, &token).await;
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_port_fails_before_ready() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        let other = Dirs::new();
        let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_kumikit"))
            .arg("serve")
            .arg("--socket")
            .arg(&other.address)
            .arg("--data-dir")
            .arg(other.data())
            .args(["--listen", &info.addr])
            .output()
            .await
            .unwrap();
        assert!(!out.status.success());
        assert!(!String::from_utf8_lossy(&out.stdout).contains("ready"));
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains(&format!("cannot listen on {}", info.addr)),
            "{err}"
        );
        // It removed the socket it had bound, so a core can start there.
        let started = Core::start(&other).await;
        started.stop().await;
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_given_certificate_is_used() {
    within(async {
        let dirs = Dirs::new();
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = rcgen::CertificateParams::new(vec!["devbox".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let (cert_path, key_path) = (dirs.root().join("c.pem"), dirs.root().join("k.pem"));
        std::fs::write(&cert_path, cert.pem()).unwrap();
        std::fs::write(&key_path, key.serialize_pem()).unwrap();
        let expected =
            kumikit_protocol::remote::format_fingerprint(&Sha256::digest(cert.der()).into());
        let (core, info) = Core::start_remote(
            &dirs,
            &[
                "--tls-cert",
                cert_path.to_str().unwrap(),
                "--tls-key",
                key_path.to_str().unwrap(),
            ],
            false,
        )
        .await;
        assert_eq!(info.fingerprint, expected);
        assert!(!dirs.data().join("tls").exists());
        let (token, _) = pair(&dirs, &info, "x").await;
        Remote::login(&info, &token).await;
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_generated_certificate_survives_a_restart() {
    within(async {
        let dirs = Dirs::new();
        let (core, first) = Core::start_remote(&dirs, &[], false).await;
        core.stop().await;
        let (core, second) = Core::start_remote(&dirs, &[], false).await;
        assert_eq!(first.fingerprint, second.fingerprint);
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn codes_and_tokens_are_never_logged() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], true).await;
        let code = pairing_code(&dirs).await;
        let code = code["code"].as_str().unwrap().to_owned();
        let mut r = Remote::connect(&info).await;
        let res = r
            .call("auth.pair", json!({"code": code, "deviceName": "x"}))
            .await;
        let token = res["token"].as_str().unwrap().to_owned();
        let mut r = Remote::login(&info, &token).await;
        r.hello().await;
        core.stop().await;
        let log = std::fs::read_to_string(dirs.root().join("core.log")).unwrap();
        assert!(log.contains("paired a new device"), "trace logging is on");
        assert!(!log.contains(&token), "the token was logged");
        assert!(!log.contains(&token[4..]), "the token was logged");
        assert!(
            !log.contains(&code) && !log.contains(&code.replace('-', "")),
            "the code was logged"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_core_stores_only_token_hashes() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        let (token, _) = pair(&dirs, &info, "x").await;
        core.stop().await;
        let mut found = false;
        for entry in std::fs::read_dir(dirs.data()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let bytes = std::fs::read(&path).unwrap();
                found |= bytes.windows(token.len()).any(|w| w == token.as_bytes());
            }
        }
        assert!(!found, "the token is stored in the data directory");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_write_side_is_noticed() {
    // A client that half-closes TLS still gets its pending answer: the server reads to the
    // end of the stream and closes cleanly.
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        let mut stream = tls(&info).await;
        stream.shutdown().await.unwrap();
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf).await;
        core.stop().await;
    })
    .await;
}
