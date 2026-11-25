//! Provider switching and bounded conversation handoff over the protocol (ADR 0029),
//! exercising a real `kumikit` core binary, two WASM providers (including `provider-acp`
//! backed by `fake-acp-agent`), transcript mode, cancelled-first-turn retry, and summary
//! mode.

use std::future::Future;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::support::{Client, Core, DEADLINE, Dirs, install, kkp, probe_kkp, provider_acp_kkp};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

fn bin_dir() -> PathBuf {
    Path::new(kumikit_test_plugins::FAKE_ACP_AGENT)
        .parent()
        .unwrap()
        .to_owned()
}

fn write_transcript(path: &Path, lines: &[Value]) {
    std::fs::write(
        path,
        lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn switch_from_probe_to_acp_provider_delivers_earlier_exchange() {
    within(async {
        let dirs = Dirs::new();
        let transcript = dirs.root().join("acp-handoff.jsonl");
        // The ACP agent checks that its first `session/prompt` contains the handoff header,
        // the earlier user message and assistant reply, and the new user prompt.
        let lines = [
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1}}}),
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "acp-s1"}}}),
            json!({
                "from": "client",
                "message": {"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {}},
                "contains": [
                    "[Conversation handoff from test.probe/probe (echo)]",
                    "hello from first agent",
                    "continue on acp"
                ]
            }),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "acp-s1", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "acp took over"}}}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 3, "result": {}}}),
        ];
        write_transcript(&transcript, &lines);

        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        install(&mut c, &provider_acp_kkp(&dirs)).await;

        let path_toml = toml::Value::String(transcript.to_string_lossy().into_owned());
        let manifest = format!(
            r#"
id = "test.acp-handoff"
name = "ACP handoff"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [
  {{ id = "agent", name = "ACP Agent", acp = {{ command = "fake-acp-agent", args = ["replay", {path_toml}] }} }},
]
"#
        );
        install(&mut c, &kkp(&dirs, "acp-handoff", &manifest, &[])).await;

        let ws = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]["id"]
            .clone();
        let thread = c
            .call(
                "thread.create",
                json!({ "workspace": ws, "provider": "test.probe/probe", "model": "echo" }),
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

        // Turn 1 on test.probe/probe.
        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "hello from first agent" } }),
        )
        .await;
        let t1 = c.events_until("turn.ended").await;
        assert_eq!(t1[0]["kind"], "turn.started");
        assert_eq!(t1[0]["data"]["provider"], "test.probe/probe");
        assert_eq!(t1[0]["data"]["model"], "echo");

        // Switch provider to the ACP agent.
        let switched = c
            .call(
                "thread.update",
                json!({
                    "thread": thread,
                    "provider": "test.acp-handoff/agent",
                    "handoff": "transcript"
                }),
            )
            .await;
        assert_eq!(switched["thread"]["provider"], "test.acp-handoff/agent");
        assert_eq!(switched["thread"]["model"], "default");

        let sw_ev = c.events_until("thread.switched").await;
        let sw = sw_ev.last().unwrap();
        assert_eq!(sw["data"]["from"]["provider"], "test.probe/probe");
        assert_eq!(sw["data"]["to"]["provider"], "test.acp-handoff/agent");
        assert_eq!(sw["data"]["handoff"], "transcript");

        // Turn 2 on the ACP agent: fake-acp-agent asserts the handoff arrived in session/prompt.
        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "continue on acp" } }),
        )
        .await;
        let t2 = c.events_until("turn.ended").await;
        assert!(
            t2.iter().any(|e| e["kind"] == "handoff.delivered"
                && e["data"]["from"]["provider"] == "test.probe/probe"
                && e["data"]["to"]["provider"] == "test.acp-handoff/agent"
                && e["data"]["messages"] == 2
                && e["data"]["truncated"] == false),
            "expected handoff.delivered in {t2:?}"
        );
        assert!(
            t2.iter()
                .any(|e| e["kind"] == "text.done" && e["data"]["text"] == "acp took over"),
            "expected ACP reply in {t2:?}"
        );

        let snap = c.call("thread.get", json!({ "thread": thread })).await;
        assert_eq!(snap["turns"][0]["provider"], "test.probe/probe");
        assert_eq!(snap["turns"][1]["provider"], "test.acp-handoff/agent");
        core.stop().await;
    })
    .await;
}
