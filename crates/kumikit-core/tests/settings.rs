//! `secret.*` and `settings.*` against the real binary (`docs/spec/03-kernel.md#plugin-settings`):
//! secrets are write-only and never logged, settings are validated against the plugin's
//! schema and its secret fields stay out of them.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use serde_json::{Value, json};
use support::{Client, Core, DEADLINE, Dirs, PROBE_MANIFEST, install, kkp};

const TOKEN: &str = "123456:settings-test-token-value";

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

/// The probe, with a settings schema that has a secret field and the secret granted.
fn probe_with_settings(dirs: &Dirs) -> std::path::PathBuf {
    let manifest = PROBE_MANIFEST
        .replace(
            "spawn = [\"fake-acp-agent\"]",
            "spawn = [\"fake-acp-agent\"]\nsecrets = [\"probe-token\"]",
        )
        .replace(
            "[contributes]",
            "[contributes]\nsettings = \"settings.schema.json\"",
        );
    let schema = json!({
        "type": "object",
        "properties": {
            "token": { "type": "string", "title": "Token", "x-secret": "probe-token" },
            "limit": { "type": "integer", "minimum": 1, "default": 5 },
            "greeting": { "type": "string" }
        }
    })
    .to_string();
    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    kkp(
        dirs,
        "probe-settings",
        &manifest,
        &[
            ("plugin.wasm", &wasm),
            ("settings.schema.json", schema.as_bytes()),
        ],
    )
}

async fn error_code(c: &mut Client, method: &str, params: Value) -> String {
    let r = c.request(method, params).await;
    r["error"]["data"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_are_write_only_and_redacted_from_the_log() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start_logged(&dirs, None).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_with_settings(&dirs)).await;

        let listed = c.call("secret.list", json!({})).await;
        assert_eq!(
            listed["secrets"],
            json!([{ "name": "probe-token", "set": false, "plugins": ["test.probe"] }])
        );
        c.call("secret.set", json!({ "name": "probe-token", "value": TOKEN })).await;
        let listed = c.call("secret.list", json!({})).await;
        assert_eq!(listed["secrets"][0]["set"], true);
        assert!(!listed.to_string().contains(TOKEN));
        assert_eq!(
            error_code(&mut c, "secret.set", json!({ "name": "no spaces", "value": "x" })).await,
            "invalid_params"
        );

        // The probe reads the secret and logs it; the core's log must not show it.
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
        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "log-secret probe-token" } }),
        )
        .await;
        let ended = c.events_until("turn.ended").await;
        assert_eq!(ended.last().unwrap()["data"]["outcome"], "done", "{ended:?}");
        core.stop().await;
        let log = dirs.log();
        assert!(log.contains("probe read [secret]"), "the probe's line is missing:\n{log}");
        assert!(!log.contains(TOKEN), "the token reached the log:\n{log}");

        // A deleted secret is not set any more.
        let _core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        c.call("secret.delete", json!({ "name": "probe-token" })).await;
        assert_eq!(c.call("secret.list", json!({})).await["secrets"][0]["set"], false);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_are_validated_and_keep_secrets_out() {
    within(async {
        let dirs = Dirs::new();
        let _core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_with_settings(&dirs)).await;
        let p = json!({ "plugin": "test.probe" });

        let schema = c.call("settings.schema", p.clone()).await;
        assert_eq!(
            schema["schema"]["properties"]["token"]["x-secret"],
            "probe-token"
        );
        assert_eq!(
            c.call("settings.get", p.clone()).await["values"],
            json!({ "limit": 5 })
        );

        c.call(
            "settings.set",
            json!({ "plugin": "test.probe", "values": { "greeting": "hi", "limit": 2 } }),
        )
        .await;
        assert_eq!(
            c.call("settings.get", p.clone()).await["values"],
            json!({ "greeting": "hi", "limit": 2 })
        );
        // The plugin reads the same values through `kumikit:host/settings`.
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
        c.call("thread.send", json!({ "thread": thread, "input": { "text": "settings" } }))
            .await;
        let events = c.events_until("turn.ended").await;
        let done = events.iter().find(|e| e["kind"] == "text.done").expect("a reply");
        let seen: Value = serde_json::from_str(done["data"]["text"].as_str().unwrap()).unwrap();
        assert_eq!(seen, json!({ "greeting": "hi", "limit": 2 }));

        for bad in [json!({ "limit": 0 }), json!({ "token": "x" }), json!([1])] {
            assert_eq!(
                error_code(
                    &mut c,
                    "settings.set",
                    json!({ "plugin": "test.probe", "values": bad })
                )
                .await,
                "invalid_params",
                "{bad}"
            );
        }
        assert_eq!(
            error_code(&mut c, "settings.get", json!({ "plugin": "test.missing" })).await,
            "not_found"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_secret_field_for_an_undeclared_secret_is_refused_at_install() {
    within(async {
        let dirs = Dirs::new();
        let _core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        let manifest =
            PROBE_MANIFEST.replace("[contributes]", "[contributes]\nsettings = \"s.json\"");
        let schema = r#"{"type":"object","properties":{"t":{"type":"string","x-secret":"other"}}}"#;
        let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
        let pkg = kkp(
            &dirs,
            "bad",
            &manifest,
            &[("plugin.wasm", &wasm), ("s.json", schema.as_bytes())],
        );
        let r = c
            .request("plugin.install", json!({ "path": pkg.to_str().unwrap() }))
            .await;
        assert_eq!(r["error"]["data"]["code"], "invalid_params", "{r}");
        assert!(r.to_string().contains("not in permissions.secrets"), "{r}");
    })
    .await;
}
