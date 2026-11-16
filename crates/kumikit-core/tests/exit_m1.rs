//! M1 exit criteria (`docs/spec/12-roadmap.md#m1--plugin-host`): an ACP agent runs
//! through provider-acp with no provider-specific code in the kernel, and a plugin that
//! calls an unpermitted host function gets `permission_denied`.
//!
//! The agent is `fake-acp-agent replay`, which plays recorded Claude Code transcripts
//! (`plugins/provider-acp/tests/transcripts`). Its provider is a manifest-only plugin
//! installed with the real CLI. The events of each turn must match what the adapter's
//! replay test expects (`plugins/provider-acp/tests/expected`), plus the kernel's own.
//! `scripts/check-plugin-names.sh` checks that `crates/` names no plugin.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{
    Client, Core, DEADLINE, Dirs, PROBE, kinds, kkp, plugin_cli, probe_kkp, provider_acp_kkp,
};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

fn acp_dir() -> PathBuf {
    Path::new(kumikit_test_plugins::PROVIDER_ACP_DIR).to_path_buf()
}

fn transcript(name: &str) -> String {
    let p = acp_dir()
        .join("tests/transcripts")
        .join(format!("{name}.jsonl"));
    std::fs::canonicalize(p)
        .unwrap()
        .to_str()
        .unwrap()
        .replace('\\', "\\\\")
}

/// A manifest-only plugin: one provider per recorded scenario, each replaying its
/// transcript through provider-acp.
fn declarative_manifest() -> String {
    let entry = |id: &str, file: &str, mode: Option<&str>| {
        let mode = mode.map_or(String::new(), |m| format!(", mode = \"{m}\""));
        format!(
            "{{ id = \"{id}\", name = \"{id}\", acp = {{ command = \"fake-acp-agent\", args = [\"replay\", \"{}\"]{mode} }} }}",
            transcript(file)
        )
    };
    format!(
        r#"
id = "test.recorded-claude"
name = "Recorded Claude Code"
version = "0.1.0"
kind = ["provider"]
api = "^0.1"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [
  {},
  {},
  {},
  {},
]
"#,
        entry("text", "claude-text", None),
        entry("allow", "claude-tool-allow", Some("default")),
        entry("deny", "claude-tool-deny", Some("default")),
        entry("cancel", "claude-cancel", None),
    )
}

/// The kernel event kinds a turn must produce: the adapter's expected events (from its
/// replay test) between the kernel's `turn.started` and `turn.ended`.
fn expected_kinds(name: &str) -> Vec<String> {
    let path = acp_dir()
        .join("tests/expected")
        .join(format!("{name}.json"));
    let log: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut out = vec!["turn.started".to_owned()];
    for e in log {
        if e.get("approve").is_some() {
            out.push("approval.requested".into());
            out.push("approval.resolved".into());
        } else if let Some(kind) = e["event"].as_str() {
            out.push(match kind {
                "plugin" => "plugin.event".to_owned(),
                k => k.replacen('_', ".", 1),
            });
        }
    }
    out.push("turn.ended".into());
    out
}

struct Env {
    dirs: Dirs,
    core: Core,
    client: Client,
    workspace: Value,
}

async fn env() -> Env {
    let dirs = Dirs::new();
    let agent_dir = Path::new(kumikit_test_plugins::FAKE_ACP_AGENT)
        .parent()
        .unwrap();
    let (core, _) = Core::start_with_path(&dirs, Some(agent_dir)).await;
    let mut client = Client::hello(&dirs.address).await;

    // Install with the real CLI, against the running core.
    let acp = provider_acp_kkp(&dirs);
    let (ok, out, err) = plugin_cli(&dirs, &["add", acp.to_str().unwrap(), "--yes"]).await;
    assert!(ok, "{out}{err}");
    assert!(out.contains("needs no permissions"), "{out}");
    let note = client.notification("core.pluginsChanged").await;
    assert_eq!(note["plugins"][0]["id"], "kumikit.provider-acp");

    let declarative = kkp(&dirs, "recorded-claude", &declarative_manifest(), &[]);
    let (ok, out, err) = plugin_cli(&dirs, &["add", declarative.to_str().unwrap(), "--yes"]).await;
    assert!(ok, "{out}{err}");
    assert!(
        out.contains("can start the `fake-acp-agent` program"),
        "{out}"
    );
    let note = client.notification("core.pluginsChanged").await;
    let providers: Vec<&Value> = note["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["contributes"]["providers"].as_array().unwrap())
        .collect();
    assert_eq!(providers.len(), 4, "{note}");

    let ws = client
        .call("workspace.open", json!({"path": dirs.workspace()}))
        .await;
    Env {
        workspace: ws["workspace"]["id"].clone(),
        dirs,
        core,
        client,
    }
}

impl Env {
    async fn thread(&mut self, provider: &str, model: &str) -> (String, String) {
        let t = self
            .client
            .call(
                "thread.create",
                json!({"workspace": self.workspace, "provider": provider, "model": model}),
            )
            .await;
        let id = t["thread"]["id"].as_str().unwrap().to_owned();
        let topic = format!("thread:{id}");
        self.client
            .call("events.subscribe", json!({"topic": topic, "after": 0}))
            .await;
        (id, topic)
    }

    async fn send(&mut self, thread: &str, text: &str) {
        self.client
            .call(
                "thread.send",
                json!({"thread": thread, "input": {"text": text}}),
            )
            .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_agent_runs_through_provider_acp() {
    within(async {
        let mut e = env().await;

        // A plain reply.
        let (t, _) = e.thread("test.recorded-claude/text", "default").await;
        e.send(&t, "Reply with exactly the words: hello from kumikit.")
            .await;
        let events = e.client.events_until("turn.ended").await;
        assert_eq!(kinds(&events), expected_kinds("claude-text"));
        assert_eq!(events.last().unwrap()["data"]["outcome"], "done");
        let done = events.iter().find(|ev| ev["kind"] == "text.done").unwrap();
        assert_eq!(done["data"]["text"], "hello from kumikit");

        // A file edit the user allows.
        let (t, _) = e.thread("test.recorded-claude/allow", "default").await;
        e.send(&t, "Create hello.txt").await;
        let mut events = e.client.events_until("approval.requested").await;
        let approval = events.last().unwrap()["data"]["approval"]["id"].clone();
        e.client
            .call(
                "thread.respondApproval",
                json!({"approval": approval, "decision": "allow"}),
            )
            .await;
        events.extend(e.client.events_until("turn.ended").await);
        assert_eq!(kinds(&events), expected_kinds("claude-tool-allow"));
        assert_eq!(events.last().unwrap()["data"]["outcome"], "done");
        let changed = events
            .iter()
            .find(|ev| ev["kind"] == "file.changed")
            .unwrap();
        assert_eq!(changed["data"]["path"], "hello.txt");

        // A command the user denies.
        let (t, _) = e.thread("test.recorded-claude/deny", "default").await;
        e.send(&t, "Run touch denied.txt").await;
        let mut events = e.client.events_until("approval.requested").await;
        let approval = events.last().unwrap()["data"]["approval"]["id"].clone();
        e.client
            .call(
                "thread.respondApproval",
                json!({"approval": approval, "decision": "deny"}),
            )
            .await;
        events.extend(e.client.events_until("turn.ended").await);
        assert_eq!(kinds(&events), expected_kinds("claude-tool-deny"));
        let finished = events
            .iter()
            .find(|ev| ev["kind"] == "tool.finished")
            .unwrap();
        assert_eq!(finished["data"]["ok"], false);

        // A cancel mid-answer reaches the agent as session/cancel.
        let (t, _) = e.thread("test.recorded-claude/cancel", "default").await;
        e.send(&t, "Write a long essay").await;
        e.client.events_until("turn.started").await;
        e.client.call("thread.cancel", json!({"thread": t})).await;
        let events = e.client.events_until("turn.ended").await;
        assert_eq!(events.last().unwrap()["data"]["outcome"], "cancelled");

        e.core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unpermitted_host_call_gets_permission_denied() {
    within(async {
        let mut e = env().await;
        let probe = probe_kkp(&e.dirs);
        let (ok, out, err) = plugin_cli(&e.dirs, &["add", probe.to_str().unwrap(), "--yes"]).await;
        assert!(ok, "{out}{err}");
        // The probe may start only `fake-acp-agent`; it asks for `sh`.
        let (t, _) = e.thread(PROBE, "probe").await;
        e.send(&t, "spawn sh").await;
        let events = e.client.events_until("turn.ended").await;
        let end = &events.last().unwrap()["data"];
        assert_eq!(end["outcome"], "failed");
        let error = end["error"].as_str().unwrap();
        assert!(error.contains("not in permissions.spawn"), "{error}");
        e.core.stop().await;
    })
    .await;
}

/// The live smoke test for provider-acp (AGENTS.md, Tests): real Claude Code through
/// `claude-agent-acp`. Needs `npm install -g @agentclientprotocol/claude-agent-acp` and
/// a Claude login, uses the network and costs a little. Run with
/// `cargo test -p kumikit-core --test exit_m1 -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: needs claude-agent-acp, a Claude login and the network"]
async fn live_claude_code_smoke() {
    within(async {
        let dirs = Dirs::new();
        let (core, _) = Core::start_with_path(&dirs, None).await;
        let mut client = Client::hello(&dirs.address).await;
        let acp = provider_acp_kkp(&dirs);
        let (ok, out, err) = plugin_cli(&dirs, &["add", acp.to_str().unwrap(), "--yes"]).await;
        assert!(ok, "{out}{err}");
        let example = std::fs::read_to_string(acp_dir().join("examples/claude-code/plugin.toml")).unwrap();
        let claude = kkp(&dirs, "claude-code", &example, &[]);
        let (ok, out, err) = plugin_cli(&dirs, &["add", claude.to_str().unwrap(), "--yes"]).await;
        assert!(ok, "{out}{err}");

        let ws = client.call("workspace.open", json!({"path": dirs.workspace()})).await;
        let t = client
            .call(
                "thread.create",
                json!({"workspace": ws["workspace"]["id"], "provider": "community.claude-code/claude"}),
            )
            .await;
        let id = t["thread"]["id"].as_str().unwrap().to_owned();
        client
            .call("events.subscribe", json!({"topic": format!("thread:{id}"), "after": 0}))
            .await;
        client
            .call(
                "thread.send",
                json!({"thread": id, "input": {"text": "Reply with exactly the word: pong. Do not use any tools."}}),
            )
            .await;
        let events = client.events_until("turn.ended").await;
        assert_eq!(events.last().unwrap()["data"]["outcome"], "done", "{events:?}");
        let done = events.iter().find(|ev| ev["kind"] == "text.done").unwrap();
        assert!(done["data"]["text"].as_str().unwrap().to_lowercase().contains("pong"));
        core.stop().await;
    })
    .await;
}
