//! The browser client's routes (`docs/spec/06-client.md#browser-client`): the UI files with
//! their CSP, pairing into an HttpOnly cookie, the same-origin rules, the cookie-authenticated
//! WebSocket, and signing out. Loopback only.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::support;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use support::{Client, Core, DEADLINE, Dirs, RemoteInfo, tls};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// One HTTP/1.1 request over the pinned TLS connection.
async fn http(
    info: &RemoteInfo,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Reply {
    let mut s = tls(info).await;
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
        info.addr,
        body.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw).await;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    Reply {
        status,
        headers,
        body: body.to_owned(),
    }
}

fn origin(info: &RemoteInfo) -> String {
    format!("https://{}", info.addr)
}

async fn web_core(dirs: &Dirs) -> (Core, RemoteInfo) {
    let ui = dirs.root().join("ui");
    std::fs::create_dir_all(ui.join("assets")).unwrap();
    std::fs::write(
        ui.join("index.html"),
        "<!doctype html><title>Kumikit</title>",
    )
    .unwrap();
    std::fs::write(ui.join("assets/app.js"), "export {};").unwrap();
    std::fs::write(dirs.root().join("secret.txt"), "outside").unwrap();
    std::fs::create_dir_all(ui.join("ui-plugins/test.theme")).unwrap();
    std::fs::write(
        ui.join("ui-plugins/test.theme/plugin.toml"),
        "id = \"test.theme\"\nname = \"T\"\nversion = \"1.0.0\"\nkind = [\"theme\"]\napi = \"^0.1\"\n",
    )
    .unwrap();
    let panel = ui.join("ui-plugins/test.panel");
    std::fs::create_dir_all(&panel).unwrap();
    std::fs::write(
        panel.join("plugin.toml"),
        "id = \"test.panel\"\nname = \"P\"\nversion = \"1.0.0\"\nkind = [\"ui\"]\napi = \"^0.1\"\n\
         [ui]\nentry = \"index.html\"\nisolation = \"iframe\"\nnetwork = [\"api.example.com\"]\n",
    )
    .unwrap();
    std::fs::write(
        panel.join("index.html"),
        "<!doctype html><title>panel</title>",
    )
    .unwrap();
    Core::start_remote(dirs, &["--web", ui.to_str().unwrap()], false).await
}

async fn code(dirs: &Dirs) -> String {
    let mut local = Client::hello(&dirs.address).await;
    local.call("auth.pairing.create", json!({})).await["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Pairs through the page route; returns the device cookie (`kk_device=…`).
async fn pair(dirs: &Dirs, info: &RemoteInfo) -> String {
    let body = json!({"code": code(dirs).await, "deviceName": "Firefox"}).to_string();
    let o = origin(info);
    let r = http(
        info,
        "POST",
        "/auth/pair",
        &[("Origin", &o), ("Content-Type", "application/json")],
        &body,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    r.header("set-cookie")
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

async fn browser_ws(
    info: &RemoteInfo,
    origin: &str,
    cookie: Option<&str>,
) -> Result<support::Ws, String> {
    let mut req = info.url.as_str().into_client_request().unwrap();
    req.headers_mut().insert("origin", origin.parse().unwrap());
    if let Some(c) = cookie {
        req.headers_mut().insert("cookie", c.parse().unwrap());
    }
    tokio_tungstenite::client_async(req, tls(info).await)
        .await
        .map(|(ws, _)| ws)
        .map_err(|e| e.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn without_web_there_are_no_web_routes() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        assert_eq!(http(&info, "GET", "/", &[], "").await.status, 404);
        assert_eq!(
            http(&info, "GET", "/auth/status", &[], "").await.status,
            404
        );
        let err = browser_ws(&info, &origin(&info), None).await.unwrap_err();
        assert!(err.contains("403"), "{err}");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ui_is_served_with_a_strict_csp_and_nothing_outside_it() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = web_core(&dirs).await;
        let r = http(&info, "GET", "/", &[], "").await;
        assert_eq!(r.status, 200);
        assert!(r.body.contains("<title>Kumikit</title>"));
        let csp = r.header("content-security-policy").unwrap();
        assert!(
            csp.contains("frame-ancestors 'none'") && csp.contains("script-src 'self'"),
            "{csp}"
        );
        assert!(!csp.contains("unsafe"), "{csp}");
        assert_eq!(r.header("x-frame-options"), Some("DENY"));
        assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
        let js = http(&info, "GET", "/assets/app.js", &[], "").await;
        assert_eq!(js.status, 200);
        assert!(js.header("content-type").unwrap().contains("javascript"));
        for bad in [
            "/../secret.txt",
            "/assets/../../secret.txt",
            "/%2e%2e/secret.txt",
        ] {
            let r = http(&info, "GET", bad, &[], "").await;
            assert!(
                !r.body.contains("outside"),
                "{bad} reached a file outside the UI"
            );
        }
        assert_eq!(http(&info, "PUT", "/", &[], "").await.status, 405);
        let list = http(&info, "GET", "/ui-plugins/index.json", &[], "").await;
        let v: Value = serde_json::from_str(&list.body).unwrap();
        let theme = v
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "test.theme")
            .unwrap();
        assert_eq!(theme["source"], "bundled");
        assert_eq!(theme["manifest"]["kind"][0], "theme");
        let toml = http(&info, "GET", "/ui-plugins/test.theme/plugin.toml", &[], "").await;
        assert_eq!(toml.status, 200);
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_from_the_page_sets_an_http_only_cookie_and_nothing_else() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = web_core(&dirs).await;
        let status = http(&info, "GET", "/auth/status", &[], "").await;
        let v: Value = serde_json::from_str(&status.body).unwrap();
        assert_eq!(v["paired"], false);
        assert!(v["core"]["name"].is_string());

        // Other sites, forms and missing origins are refused before the code is looked at.
        let c = code(&dirs).await;
        let body = json!({"code": c, "deviceName": "x"}).to_string();
        let o = origin(&info);
        for headers in [
            vec![("Content-Type", "application/json")],
            vec![
                ("Origin", "https://evil.example"),
                ("Content-Type", "application/json"),
            ],
            vec![
                ("Origin", o.as_str()),
                ("Content-Type", "application/x-www-form-urlencoded"),
            ],
            vec![("Origin", o.as_str()), ("Content-Type", "text/plain")],
        ] {
            let r = http(&info, "POST", "/auth/pair", &headers, &body).await;
            assert_eq!(r.status, 403, "{headers:?}");
        }
        let wrong = json!({"code": "0000-0000", "deviceName": "x"}).to_string();
        let r = http(
            &info,
            "POST",
            "/auth/pair",
            &[("Origin", &o), ("Content-Type", "application/json")],
            &wrong,
        )
        .await;
        assert_eq!(r.status, 401);

        // The right request: the code was not burnt by the refused ones.
        let r = http(
            &info,
            "POST",
            "/auth/pair",
            &[("Origin", &o), ("Content-Type", "application/json")],
            &body,
        )
        .await;
        assert_eq!(r.status, 200, "{}", r.body);
        let cookie = r.header("set-cookie").unwrap();
        for part in [
            "kk_device=kkd_",
            "HttpOnly",
            "Secure",
            "SameSite=Strict",
            "Path=/",
        ] {
            assert!(cookie.contains(part), "{cookie}");
        }
        assert!(
            !r.body.contains("kkd_"),
            "the token is in the body: {}",
            r.body
        );
        let cookie = cookie.split(';').next().unwrap();
        let status = http(&info, "GET", "/auth/status", &[("Cookie", cookie)], "").await;
        assert!(status.body.contains("\"paired\":true"), "{}", status.body);
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_socket_takes_the_cookie_only_from_this_cores_page() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = web_core(&dirs).await;
        let cookie = pair(&dirs, &info).await;

        let err = browser_ws(&info, &origin(&info), None).await.unwrap_err();
        assert!(err.contains("401"), "{err}");
        let err = browser_ws(&info, "https://evil.example", Some(&cookie)).await.unwrap_err();
        assert!(err.contains("403"), "{err}");

        // Authenticated by the cookie: the first message is core.hello.
        let mut ws = browser_ws(&info, &origin(&info), Some(&cookie)).await.unwrap();
        ws.send(Message::text(
            json!({"jsonrpc":"2.0","id":1,"method":"core.hello","params":{"protocol":"0.12","client":{"name":"t","version":"0"}}}).to_string(),
        ))
        .await
        .unwrap();
        let hello = loop {
            if let Some(Ok(Message::Text(t))) = ws.next().await {
                break serde_json::from_str::<Value>(t.as_str()).unwrap();
            }
        };
        assert_eq!(hello["result"]["protocol"], "0.12", "{hello}");

        // Signing out revokes the device: its socket closes and the cookie stops working.
        let o = origin(&info);
        let r = http(&info, "POST", "/auth/logout", &[("Origin", &o), ("Content-Type", "application/json"), ("Cookie", &cookie)], "{}").await;
        assert_eq!(r.status, 200);
        assert!(r.header("set-cookie").unwrap().contains("Max-Age=0"));
        let closed = loop {
            match ws.next().await {
                Some(Ok(Message::Close(f))) => break f.map(|f| u16::from(f.code)),
                Some(Ok(_)) => {}
                _ => break None,
            }
        };
        assert_eq!(closed, Some(4001));
        let err = browser_ws(&info, &origin(&info), Some(&cookie)).await.unwrap_err();
        assert!(err.contains("401"), "{err}");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn web_needs_an_index_and_listen() {
    within(async {
        let dirs = Dirs::new();
        let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_kumikit"))
            .args(["serve", "--socket", &dirs.address])
            .arg("--data-dir")
            .arg(dirs.data())
            .args(["--listen", "127.0.0.1:0", "--web"])
            .arg(dirs.root())
            .output()
            .await
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("no index.html"));
        let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_kumikit"))
            .args(["serve", "--web"])
            .arg(dirs.root())
            .output()
            .await
            .unwrap();
        assert!(!out.status.success(), "--web without --listen");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panels_files_carry_its_own_csp_and_only_this_core_may_frame_them() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = web_core(&dirs).await;
        let own = origin(&info);
        let r = http(&info, "GET", "/ui-plugins/test.panel/index.html", &[], "").await;
        assert_eq!(r.status, 200);
        let csp = r.header("content-security-policy").unwrap();
        assert!(
            csp.contains(&format!("script-src {own}/ui-plugins/test.panel/;")),
            "{csp}"
        );
        assert!(
            csp.contains("connect-src https://api.example.com;"),
            "{csp}"
        );
        assert!(csp.contains(&format!("frame-ancestors {own}")), "{csp}");
        assert!(!csp.contains("'self'") && !csp.contains("unsafe"), "{csp}");
        assert_eq!(r.header("x-frame-options"), Some("SAMEORIGIN"));
        assert_eq!(r.header("access-control-allow-origin"), Some("null"));

        // Other plugins and the app keep the page's CSP and may not be framed.
        let theme = http(&info, "GET", "/ui-plugins/test.theme/plugin.toml", &[], "").await;
        let csp = theme.header("content-security-policy").unwrap();
        assert!(
            csp.contains("frame-ancestors 'none'") && csp.contains("frame-src 'self'"),
            "{csp}"
        );
        assert_eq!(theme.header("x-frame-options"), Some("DENY"));
        assert_eq!(theme.header("access-control-allow-origin"), None);

        core.stop().await;
    })
    .await;
}
