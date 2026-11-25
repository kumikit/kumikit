//! M7 exit tests over the protocol against the real `kumikit` core binary:
//! 1. Durable queue survives a core restart as `restarted` held and resumes in order.
//! 2. Provider switch (`test.probe/probe` -> `provider-acp` + `fake-acp-agent`) resends the
//!    bounded handoff when the first turn on the new provider is cancelled, and records
//!    per-turn provider/model attribution.
//! 3. `workspace.files.list`, `after_command` with `onlyOnFailure`, ACP
//!    `available_commands_update` -> `commands.available`, and ACP `429` -> `limit.reached`
//!    (`status: limited`, `queueHold: limited`, resumed via `thread.queue.resume`).

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
async fn durable_queue_held_across_core_restart_and_resumes_in_order() {
    within(async {
        let dirs = Dirs::new();
        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;

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

        // Complete one turn first so the provider session is persisted before we
        // interrupt the next turn with a core restart.
        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "kv" } }),
        )
        .await;
        c.events_until("turn.ended").await;

        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "hang" } }),
        )
        .await;
        c.events_until("turn.started").await;

        let first = c
            .call(
                "thread.send",
                json!({ "thread": thread, "input": { "text": "kv" } }),
            )
            .await;
        let second = c
            .call(
                "thread.send",
                json!({ "thread": thread, "input": { "text": "fs-own" } }),
            )
            .await;
        assert_eq!(first["queued"], true);
        assert_eq!(second["queued"], true);
        let (a, b) = (
            first["entry"].as_str().unwrap().to_owned(),
            second["entry"].as_str().unwrap().to_owned(),
        );

        // Stop and restart the core while the turn is active and two entries are queued.
        drop(c);
        core.stop().await;

        let (core2, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c2 = Client::hello(&dirs.address).await;

        let snap = c2.call("thread.get", json!({ "thread": thread })).await;
        assert_eq!(snap["thread"]["status"], "failed", "{snap}");
        assert_eq!(snap["queueHold"], "restarted", "{snap}");
        let ids: Vec<&str> = snap["queue"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![a.as_str(), b.as_str()]);

        let seq = snap["seq"].as_u64().unwrap();
        c2.call(
            "events.subscribe",
            json!({ "topic": format!("thread:{thread}"), "after": seq }),
        )
        .await;

        c2.call("thread.queue.resume", json!({ "thread": thread }))
            .await;
        let mut events = c2.events_until("turn.ended").await;
        events.extend(c2.events_until("turn.ended").await);

        let started: Vec<&str> = events
            .iter()
            .filter(|e| e["kind"] == "turn.started")
            .map(|e| e["data"]["entry"].as_str().unwrap())
            .collect();
        assert_eq!(started, vec![a.as_str(), b.as_str()]);

        let after = c2.call("thread.get", json!({ "thread": thread })).await;
        assert!(after.get("queue").is_none(), "{after}");
        assert!(after.get("queueHold").is_none(), "{after}");
        core2.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn handoff_resends_after_cancelled_first_turn_and_records_authors() {
    within(async {
        let dirs = Dirs::new();
        let transcript = dirs.root().join("acp-retry.jsonl");
        let cancel_lines = [
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1}}}),
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "acp-r1"}}}),
            // Attempt 1 receives the handoff and waits for session/cancel.
            json!({
                "from": "client",
                "message": {"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {}},
                "contains": [
                    "[Conversation handoff from test.probe/probe (echo)]",
                    "remember this context",
                    "first attempt"
                ]
            }),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "acp-r1", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "starting..."}}}}}),
            json!({"from": "client", "message": {"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": "acp-r1"}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 3, "result": {"stopReason": "cancelled"}}}),
        ];
        write_transcript(&transcript, &cancel_lines);

        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        install(&mut c, &provider_acp_kkp(&dirs)).await;

        let path_toml = toml::Value::String(transcript.to_string_lossy().into_owned());
        let manifest = format!(
            r#"
id = "test.acp-retry"
name = "ACP retry"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [
  {{ id = "agent", name = "ACP Retry Agent", acp = {{ command = "fake-acp-agent", args = ["replay", {path_toml}] }} }},
]
"#
        );
        install(&mut c, &kkp(&dirs, "acp-retry", &manifest, &[])).await;

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

        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "remember this context" } }),
        )
        .await;
        c.events_until("turn.ended").await;

        c.call(
            "thread.update",
            json!({
                "thread": thread,
                "provider": "test.acp-retry/agent",
                "handoff": "transcript"
            }),
        )
        .await;
        c.events_until("thread.switched").await;

        // First turn on the new provider gets cancelled before completing.
        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "first attempt" } }),
        )
        .await;
        c.events_until("text.delta").await;
        c.call("thread.cancel", json!({ "thread": thread })).await;
        let cancelled = c.events_until("turn.ended").await;
        assert_eq!(cancelled.last().unwrap()["data"]["outcome"], "cancelled");
        assert!(
            !cancelled.iter().any(|e| e["kind"] == "handoff.delivered"),
            "cancelled turn must not mark handoff delivered: {cancelled:?}"
        );

        // Attempt 1's cancelled session was reset by the kernel (ADR 0029); attempt 2
        // starts a fresh session and must receive the handoff again.
        let retry_lines = [
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1}}}),
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "acp-r2"}}}),
            json!({
                "from": "client",
                "message": {"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {}},
                "contains": [
                    "[Conversation handoff from test.probe/probe (echo)]",
                    "remember this context",
                    "second attempt"
                ]
            }),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "acp-r2", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "handoff succeeded"}}}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 3, "result": {"stopReason": "end_turn"}}}),
        ];
        write_transcript(&transcript, &retry_lines);

        // Second turn resends the handoff and completes.
        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "second attempt" } }),
        )
        .await;
        let done = c.events_until("turn.ended").await;
        assert!(
            done.iter().any(|e| e["kind"] == "handoff.delivered"),
            "expected handoff.delivered on retry: {done:?}"
        );
        assert!(
            done.iter()
                .any(|e| e["kind"] == "text.done" && e["data"]["text"] == "handoff succeeded")
        );

        let snap = c.call("thread.get", json!({ "thread": thread })).await;
        assert_eq!(snap["turns"][0]["provider"], "test.probe/probe");
        assert_eq!(snap["turns"][0]["model"], "echo");
        assert_eq!(snap["turns"][2]["provider"], "test.acp-retry/agent");
        assert_eq!(snap["turns"][2]["model"], "default");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn acp_commands_usage_limits_workspace_files_and_only_on_failure() {
    within(async {
        let dirs = Dirs::new();
        std::fs::write(Path::new(&dirs.workspace()).join("notes.md"), "# Notes\n").unwrap();
        std::fs::create_dir_all(Path::new(&dirs.workspace()).join(".git")).unwrap();
        std::fs::write(Path::new(&dirs.workspace()).join(".git/HEAD"), "ref: refs/heads/main\n")
            .unwrap();

        let transcript = dirs.root().join("acp-cmds-limit.jsonl");
        let lines = [
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1}}}),
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "acp-cl1"}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "acp-cl1", "update": {"sessionUpdate": "available_commands_update", "availableCommands": [{"name": "review", "description": "Review changes", "input": {"hint": "[branch]"}}]}}}}),
            // Turn 1 emits commands.available and succeeds.
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "acp-cl1", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "ready"}}}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 3, "result": {"stopReason": "end_turn"}}}),
            // Turn 2 hits a 429 rate limit error.
            json!({"from": "client", "message": {"jsonrpc": "2.0", "id": 4, "method": "session/prompt", "params": {}}}),
            json!({"from": "agent", "message": {"jsonrpc": "2.0", "id": 4, "error": {"code": 429, "message": "rate limit reached"}}}),
        ];
        write_transcript(&transcript, &lines);

        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &provider_acp_kkp(&dirs)).await;

        let path_toml = toml::Value::String(transcript.to_string_lossy().into_owned());
        let manifest = format!(
            r#"
id = "test.acp-cl"
name = "ACP commands and limits"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [
  {{ id = "agent", name = "ACP CL Agent", acp = {{ command = "fake-acp-agent", args = ["replay", {path_toml}] }} }},
]
"#
        );
        install(&mut c, &kkp(&dirs, "acp-cl", &manifest, &[])).await;

        let ws = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]["id"]
            .clone();

        // 1. workspace.files.list returns relative paths and omits `.git`.
        let listed = c
            .call("workspace.files.list", json!({ "workspace": ws, "query": "not" }))
            .await;
        let files: Vec<&str> = listed["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(files, vec!["notes.md"]);
        let all = c
            .call("workspace.files.list", json!({ "workspace": ws }))
            .await;
        assert!(
            !all["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str().unwrap().starts_with(".git")),
            "expected .git to be omitted: {all}"
        );

        let thread = c
            .call(
                "thread.create",
                json!({ "workspace": ws, "provider": "test.acp-cl/agent" }),
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

        // 2. Turn 1 receives buffered available_commands_update as commands.available.
        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "hello" } }),
        )
        .await;
        let t1 = c.events_until("turn.ended").await;
        let cmds_ev = t1
            .iter()
            .find(|e| e["kind"] == "commands.available")
            .unwrap_or_else(|| panic!("expected commands.available in {t1:?}"));
        assert_eq!(cmds_ev["data"]["commands"][0]["name"], "review");
        assert_eq!(cmds_ev["data"]["commands"][0]["hint"], "[branch]");

        // 3. Deferred after_command with onlyOnFailure=true is silently dropped on exit 0.
        let ok_argv = if cfg!(windows) {
            json!(["cmd", "/C", "exit 0"])
        } else {
            json!(["sh", "-c", "exit 0"])
        };
        c.call(
            "thread.defer",
            json!({
                "thread": thread,
                "input": { "text": "fix this" },
                "trigger": {
                    "kind": "after_command",
                    "argv": ok_argv,
                    "timeoutS": 30,
                    "onlyOnFailure": true
                }
            }),
        )
        .await;
        let removed = c.events_until("queue.removed").await;
        assert_eq!(
            removed.last().unwrap()["data"]["reason"],
            "condition_not_met"
        );

        // 4. Turn 2 hits a 429 rate limit -> limit.reached, status limited, queueHold limited.
        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "trigger limit" } }),
        )
        .await;
        let t2 = c.events_until("turn.ended").await;
        assert!(
            t2.iter().any(|e| e["kind"] == "limit.reached"),
            "expected limit.reached in {t2:?}"
        );
        // `queue.held` is appended right after `turn.ended` by `finish_run`.
        let held = c.events_until("queue.held").await;
        assert_eq!(held.last().unwrap()["data"]["reason"], "limited");
        let snap = c.call("thread.get", json!({ "thread": thread })).await;
        assert_eq!(snap["thread"]["status"], "limited", "{snap}");
        assert_eq!(snap["queueHold"], "limited", "{snap}");

        // Resuming clears the hold and returns the thread to idle.
        c.call("thread.queue.resume", json!({ "thread": thread }))
            .await;
        let resumed = c.call("thread.get", json!({ "thread": thread })).await;
        assert_eq!(resumed["thread"]["status"], "idle", "{resumed}");
        assert!(resumed.get("queueHold").is_none(), "{resumed}");

        core.stop().await;
    })
    .await;
}
