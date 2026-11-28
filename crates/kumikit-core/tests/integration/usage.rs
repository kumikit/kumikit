//! Usage statistics and plan limits through the real binary (ADR 0035,
//! `docs/spec/02-protocol.md#usage`): limits a provider reports during a turn reach the
//! `usage` topic and `usage.limits`; usage by model reaches `usage.summary`; a refresh runs
//! the plugin's reader, either the provider's own component or a declarative provider's
//! small reader component.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::support;

use serde_json::{Value, json};
use support::{Client, Core, DEADLINE, Dirs, PROBE, install, kkp, probe_kkp, provider_acp_kkp};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

/// The next event on `topic`, skipping events of other topics.
async fn next_on(c: &mut Client, topic: &str) -> Value {
    loop {
        let e = c.event().await;
        if e["topic"] == topic {
            return e;
        }
    }
}

async fn send(c: &mut Client, thread: &str, text: &str) {
    c.call(
        "thread.send",
        json!({ "thread": thread, "input": { "text": text } }),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_providers_limits_and_usage_reach_clients() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;

        let providers = c.call("provider.list", json!({})).await;
        let probe = &providers["providers"][0];
        assert_eq!(probe["id"], PROBE);
        assert_eq!(probe["readsLimits"], true);

        // Nothing reported yet.
        let none = c.call("usage.limits", json!({})).await;
        assert_eq!(none["providers"], json!([{ "provider": PROBE }]));

        let ws = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await;
        let thread = c
            .call(
                "thread.create",
                json!({ "workspace": ws["workspace"]["id"], "provider": PROBE, "model": "probe" }),
            )
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let sub = c
            .call("events.subscribe", json!({ "topic": "usage", "after": 0 }))
            .await;
        assert_eq!(sub["head"], 0);

        // Limits seen during a turn.
        send(&mut c, &thread, "limits 26").await;
        let e = next_on(&mut c, "usage").await;
        assert_eq!(e["kind"], "usage.limits");
        assert_eq!(e["data"]["provider"], PROBE);
        assert_eq!(e["data"]["limits"]["plan"], "Probe plan");
        assert_eq!(e["data"]["limits"]["windows"][0]["usedPercent"], 26.0);
        let stored = c.call("usage.limits", json!({ "provider": PROBE })).await;
        assert_eq!(stored["providers"][0], e["data"]);

        // Usage by model.
        send(&mut c, &thread, "usage-models").await;
        let e = next_on(&mut c, "usage").await;
        assert_eq!(e["kind"], "usage.recorded");
        assert_eq!(e["data"], json!({ "provider": PROBE, "thread": thread }));
        let summary = c.call("usage.summary", json!({ "days": 30 })).await;
        assert_eq!(summary["totals"]["inputTokens"], 110);
        assert_eq!(summary["totals"]["cachedInputTokens"], 60);
        assert_eq!(summary["totals"]["cost"], 1.0);
        assert_eq!(summary["threads"], 1);
        let models: Vec<&str> = summary["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["model"].as_str().unwrap())
            .collect();
        assert_eq!(models, ["probe", "probe-small"]);
        assert_eq!(summary["daily"].as_array().unwrap().len(), 1);

        // A refresh runs the reader once, then the stored snapshot comes back for a while.
        let first = c.call("usage.refresh", json!({ "provider": PROBE })).await;
        assert_eq!(first["limits"]["windows"][0]["usedPercent"], 10.0);
        let again = c.call("usage.refresh", json!({ "provider": PROBE })).await;
        assert_eq!(again["limits"]["windows"][0]["usedPercent"], 10.0);

        let bad = c.request("usage.summary", json!({ "days": 0 })).await;
        assert_eq!(bad["error"]["data"]["code"], "invalid_params", "{bad}");
        let bad = c
            .request("events.subscribe", json!({ "topic": "usage", "after": 3 }))
            .await;
        assert_eq!(bad["error"]["data"]["code"], "invalid_params", "{bad}");
        let bad = c
            .request("usage.refresh", json!({ "provider": "nope/nope" }))
            .await;
        assert_eq!(bad["error"]["data"]["code"], "not_found", "{bad}");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declarative_providers_reader_answers_refreshes() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &provider_acp_kkp(&dirs)).await;
        let wasm = std::fs::read(kumikit_test_plugins::LIMITS_WASM).unwrap();
        let limits = kkp(
            &dirs,
            "limits",
            kumikit_test_plugins::LIMITS_MANIFEST,
            &[("plugin.wasm", &wasm)],
        );
        install(&mut c, &limits).await;
        // A declarative provider without a reader, for contrast.
        let plain = kkp(
            &dirs,
            "plain",
            r#"id = "test.plain"
name = "Plain"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [{ id = "plain", name = "Plain", acp = { command = "fake-acp-agent", args = [] } }]
"#,
            &[],
        );
        install(&mut c, &plain).await;

        let providers = c.call("provider.list", json!({})).await;
        let flags: Vec<(String, Value)> = providers["providers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["id"].as_str().unwrap().to_owned(),
                    p["readsLimits"].clone(),
                )
            })
            .collect();
        assert_eq!(
            flags,
            vec![
                ("test.limits/agent".to_owned(), json!(true)),
                ("test.limits/broken".to_owned(), json!(true)),
                ("test.limits/none".to_owned(), json!(true)),
                ("test.plain/plain".to_owned(), Value::Null),
            ]
        );

        let got = c
            .call("usage.refresh", json!({ "provider": "test.limits/agent" }))
            .await;
        assert_eq!(got["provider"], "test.limits/agent");
        let window = &got["limits"]["windows"][0];
        assert_eq!(window["id"], "agent-weekly");
        assert_eq!(window["group"], "Gemini models");
        assert_eq!(window["model"], "gemini");
        assert_eq!(got["limits"]["accountUrl"], "https://example.com/plans");

        let none = c
            .call("usage.refresh", json!({ "provider": "test.limits/none" }))
            .await;
        assert!(none.get("limits").is_none(), "{none}");
        assert!(none["observed"].is_string(), "{none}");

        let broken = c
            .request("usage.refresh", json!({ "provider": "test.limits/broken" }))
            .await;
        assert_eq!(broken["error"]["data"]["code"], "plugin_failed", "{broken}");
        assert!(
            broken["error"]["message"]
                .as_str()
                .unwrap()
                .contains("the usage report failed"),
            "{broken}"
        );

        let plain = c
            .request("usage.refresh", json!({ "provider": "test.plain/plain" }))
            .await;
        assert_eq!(plain["error"]["data"]["code"], "unsupported", "{plain}");

        // A disabled provider's limits are no longer listed.
        c.call("plugin.disable", json!({ "plugin": "test.limits" }))
            .await;
        let listed = c.call("usage.limits", json!({})).await;
        assert_eq!(
            listed["providers"],
            json!([{ "provider": "test.plain/plain" }])
        );
        core.stop().await;
    })
    .await;
}
