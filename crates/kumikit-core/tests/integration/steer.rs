//! Steering over the protocol (ADR 0028), against the real binary and the probe provider,
//! whose `steer` command reads its child's stdout through `kumikit:turn/input`.

use std::future::Future;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::support::{
    Client, Core, DEADLINE, Dirs, install, kinds, kkp, probe_kkp, provider_acp_kkp,
};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

/// Where `fake-acp-agent` (which `steer` starts) lives.
fn bin_dir() -> PathBuf {
    Path::new(kumikit_test_plugins::FAKE_ACP_AGENT)
        .parent()
        .unwrap()
        .to_owned()
}

async fn thread(c: &mut Client, dirs: &Dirs, model: &str) -> String {
    let ws = c
        .call("workspace.open", json!({ "path": dirs.workspace() }))
        .await["workspace"]["id"]
        .clone();
    let thread = c
        .call(
            "thread.create",
            json!({ "workspace": ws, "provider": "test.probe/probe", "model": model }),
        )
        .await["thread"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    c.call(
        "events.subscribe",
        json!({ "topic": format!("thread:{thread}"), "after": 0 }),
    )
    .await;
    thread
}

async fn call(c: &mut Client, method: &str, thread: &str, text: &str) -> Value {
    c.call(
        method,
        json!({ "thread": thread, "input": { "text": text } }),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steer_reaches_the_agent_mid_turn() {
    within(async {
        let dirs = Dirs::new();
        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        let models = c
            .call("provider.models", json!({ "provider": "test.probe/probe" }))
            .await;
        assert_eq!(
            models["models"][0]["capabilities"],
            json!(["steer"]),
            "{models}"
        );

        let thread = thread(&mut c, &dirs, "probe").await;
        let sent = call(&mut c, "thread.send", &thread, "steer").await;
        c.events_until("turn.started").await;
        let r = call(&mut c, "thread.steer", &thread, "go left").await;
        assert_eq!(r["status"], "accepted", "{r}");
        assert_eq!(r["turn"], sent["turn"]);

        let events = c.events_until("turn.ended").await;
        assert_eq!(kinds(&events), ["input.steered", "text.done", "turn.ended"]);
        assert_eq!(events[0]["data"]["input"]["text"], "go left");
        assert_eq!(events[0]["data"]["turn"], sent["turn"]);
        assert_eq!(events[1]["data"]["text"], "steered: go left");
        assert_eq!(events[2]["data"]["outcome"], "done");

        let idle = c
            .request(
                "thread.steer",
                json!({ "thread": thread, "input": { "text": "too late" } }),
            )
            .await;
        assert_eq!(idle["error"]["data"]["code"], "conflict", "{idle}");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_model_without_steer_queues_the_message_first() {
    within(async {
        let dirs = Dirs::new();
        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        let thread = thread(&mut c, &dirs, "tool-approval").await;
        call(&mut c, "thread.send", &thread, "anything").await;
        c.events_until("approval.requested").await;
        call(&mut c, "thread.send", &thread, "second").await;
        c.events_until("queue.added").await;
        let r = call(&mut c, "thread.steer", &thread, "first").await;
        assert_eq!(r["status"], "queued", "{r}");
        let events = c.events_until("queue.added").await;
        let added = events.last().unwrap();
        // The steer goes to the front, ahead of "second".
        assert_eq!(added["data"]["entry"]["input"]["text"], "first", "{added}");
        assert_eq!(added["data"]["front"], true);
        assert_eq!(added["data"]["reason"], "not_steered");
        let snap = c.call("thread.get", json!({ "thread": thread })).await;
        let texts: Vec<&str> = snap["queue"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["input"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, ["first", "second"]);
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_plugin_steers_a_running_turn_in_a_linked_workspace() {
    within(async {
        let dirs = Dirs::new();
        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        let wasm = std::fs::read(kumikit_test_plugins::CLIENT_WASM).unwrap();
        let pkg = kkp(
            &dirs,
            "client",
            kumikit_test_plugins::CLIENT_MANIFEST,
            &[("plugin.wasm", &wasm)],
        );
        install(&mut c, &pkg).await;

        let ws = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]["id"]
            .clone();
        let thread = c
            .call(
                "thread.create",
                json!({ "workspace": ws, "provider": "test.probe/probe", "model": "probe" }),
            )
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        c.call(
            "events.subscribe",
            json!({ "topic": format!("thread:{thread}"), "after": 0 }),
        )
        .await;
        let sent = call(&mut c, "thread.send", &thread, "steer").await;
        c.events_until("turn.started").await;

        // Before linking the workspace, steering is denied.
        let denied = c
            .request(
                "command.run",
                json!({
                    "plugin": "test.client",
                    "command": "steer",
                    "args": { "thread": thread, "text": "nope" }
                }),
            )
            .await;
        assert_eq!(
            denied["error"]["data"]["code"], "permission_denied",
            "{denied}"
        );

        // Once linked, the plugin steers the running turn.
        c.call(
            "plugin.setWorkspaces",
            json!({ "plugin": "test.client", "workspaces": [ws] }),
        )
        .await;
        let r = c
            .call(
                "command.run",
                json!({
                    "plugin": "test.client",
                    "command": "steer",
                    "args": { "thread": thread, "text": "from plugin" }
                }),
            )
            .await;
        let parsed: Value = serde_json::from_str(r["message"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["status"], "accepted", "{parsed}");
        assert_eq!(parsed["turn"], sent["turn"]);

        let events = c.events_until("turn.ended").await;
        assert_eq!(kinds(&events), ["input.steered", "text.done", "turn.ended"]);
        assert_eq!(events[1]["data"]["text"], "steered: from plugin");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_agent_with_steer_opted_in_takes_mid_turn_input() {
    within(async {
        let dirs = Dirs::new();
        let transcript = dirs.root().join("acp-steer.jsonl");
        let lines = [
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1}}}),
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s1"}}}),
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "working"}}}}}),
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 4, "method": "session/prompt", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "changed course"}}}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 3, "result": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 4, "result": {}}}),
        ];
        std::fs::write(
            &transcript,
            lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();

        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &provider_acp_kkp(&dirs)).await;

        let path_toml = toml::Value::String(transcript.to_string_lossy().into_owned());
        let manifest = format!(
            r#"
id = "test.acp-steer"
name = "ACP steer"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [
  {{ id = "steerable", name = "Steerable", acp = {{ command = "fake-acp-agent", args = ["replay", {path_toml}], steer = true }} }},
  {{ id = "plain", name = "Plain", acp = {{ command = "fake-acp-agent", args = ["replay", {path_toml}] }} }},
]
"#
        );
        install(&mut c, &kkp(&dirs, "acp-steer", &manifest, &[])).await;

        let steerable = c
            .call(
                "provider.models",
                json!({ "provider": "test.acp-steer/steerable" }),
            )
            .await;
        assert_eq!(steerable["models"][0]["capabilities"], json!(["steer"]), "{steerable}");
        let plain = c
            .call(
                "provider.models",
                json!({ "provider": "test.acp-steer/plain" }),
            )
            .await;
        assert_eq!(plain["models"][0]["capabilities"], json!([]), "{plain}");

        let ws = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]["id"]
            .clone();
        let thread = c
            .call(
                "thread.create",
                json!({ "workspace": ws, "provider": "test.acp-steer/steerable", "model": "default" }),
            )
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        c.call(
            "events.subscribe",
            json!({ "topic": format!("thread:{thread}"), "after": 0 }),
        )
        .await;

        let sent = call(&mut c, "thread.send", &thread, "start").await;
        let before = c.events_until("text.delta").await;
        assert_eq!(before.last().unwrap()["data"]["text"], "working");

        let r = call(&mut c, "thread.steer", &thread, "go left").await;
        assert_eq!(r["status"], "accepted", "{r}");
        assert_eq!(r["turn"], sent["turn"]);

        let after = c.events_until("turn.ended").await;
        assert_eq!(
            kinds(&after),
            ["input.steered", "text.delta", "text.done", "turn.ended"]
        );
        assert_eq!(after[0]["data"]["input"]["text"], "go left");
        assert_eq!(after[1]["data"]["text"], "changed course");
        assert_eq!(after[2]["data"]["text"], "changed course");
        assert_eq!(after[3]["data"]["outcome"], "done");
        core.stop().await;
    })
    .await;
}
