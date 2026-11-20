//! M5 exit criteria in the core (`docs/plans/2026-11-19-m5-plugins.md`):
//! - E2 (`gateway`): an agent turn calls a read tool and an approval-gated write tool
//!   through the real MCP gateway (`provider-acp` -> `fake-acp-agent replay` -> `kumikit
//!   mcp`), while a provider without the `tools` grant receives no gateway.
//! - E5 (`schedules`): a scheduled prompt plugin sets a schedule, `schedule.list` shows
//!   its next run, `schedule.run` starts a turn attributed to the plugin, pausing stops it,
//!   and deleting removes it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{Client, Core, DEADLINE, Dirs, install, kkp, probe_kkp, provider_acp_kkp};

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

fn tool_kkp(dirs: &Dirs) -> PathBuf {
    let src = Path::new(kumikit_test_plugins::TOOL_DIR);
    let manifest = std::fs::read_to_string(src.join("plugin.toml")).unwrap();
    let wasm = std::fs::read(kumikit_test_plugins::TOOL_WASM).unwrap();
    let echo = std::fs::read(src.join("tools/echo.json")).unwrap();
    let empty = std::fs::read(src.join("tools/empty.json")).unwrap();
    std::fs::create_dir_all(dirs.root().join("exit-m5-tool-src/tools")).unwrap();
    kkp(
        dirs,
        "exit-m5-tool",
        &manifest,
        &[
            ("plugin.wasm", &wasm),
            ("tools/echo.json", &echo),
            ("tools/empty.json", &empty),
        ],
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway() {
    within(async {
        let dirs = Dirs::new();
        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &provider_acp_kkp(&dirs)).await;
        install(&mut c, &tool_kkp(&dirs)).await;

        let transcript = dirs.root().join("m5-gateway.jsonl");
        let lines = [
            json!({"from":"client","message":{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}}),
            json!({"from":"agent","message":{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}}),
            json!({"from":"client","message":{"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}}),
            json!({"from":"agent","message":{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}}),
            json!({"from":"client","message":{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{}}}),
            json!({"from":"mcp","session":"s1","call":{"name":"test_tool__echo","arguments":{"text":"from-gateway"}}}),
            json!({"from":"mcp","session":"s1","call":{"name":"test_tool__touch","arguments":{"text":"note"}}}),
            json!({"from":"agent","message":{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        std::fs::write(&transcript, format!("{lines}\n")).unwrap();
        let t = std::fs::canonicalize(&transcript)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "\\\\");

        // One provider with `tools = true` and one without.
        let with_tools = format!(
            "id = \"test.m5-agent\"\nname = \"M5 Agent\"\nversion = \"0.1.0\"\nkind = [\"provider\"]\napi = \"^0.2\"\n\
             [permissions]\nspawn = [\"fake-acp-agent\"]\ntools = true\n\
             [contributes]\nproviders = [{{ id = \"agent\", name = \"Agent\", acp = {{ command = \"fake-acp-agent\", args = [\"replay\", \"{t}\"] }} }}]\n"
        );
        let without_tools = with_tools
            .replace("test.m5-agent", "test.m5-plain")
            .replace("tools = true\n", "");
        install(&mut c, &kkp(&dirs, "m5-agent", &with_tools, &[])).await;
        install(&mut c, &kkp(&dirs, "m5-plain", &without_tools, &[])).await;

        let ws = c
            .call("workspace.open", json!({"path": dirs.workspace()}))
            .await["workspace"]["id"]
            .clone();

        // With `tools = true`: read runs directly, write waits for approval and then runs.
        let tid = c
            .call(
                "thread.create",
                json!({"workspace": ws, "provider": "test.m5-agent/agent"}),
            )
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let topic = format!("thread:{tid}");
        c.call("events.subscribe", json!({"topic": topic, "after": 0}))
            .await;
        c.call(
            "thread.send",
            json!({"thread": tid, "input": {"text": "call tools"}}),
        )
        .await;
        let mut events = c.events_until("approval.requested").await;
        let aid = events
            .last()
            .unwrap()["data"]["approval"]["id"]
            .clone();
        c.call(
            "thread.respondApproval",
            json!({"approval": aid, "decision": "allow"}),
        )
        .await;
        events.extend(c.events_until("turn.ended").await);
        assert_eq!(events.last().unwrap()["data"]["outcome"], "done");
        assert!(
            events
                .iter()
                .any(|e| e["kind"] == "tool.started" && e["data"]["source"]["tool"] == "echo"),
            "missing gateway tool.started for echo"
        );
        let text = events
            .iter()
            .find(|e| e["kind"] == "text.done")
            .unwrap()["data"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("from-gateway"), "{text}");
        assert!(text.contains("touched note"), "{text}");

        // Without `tools = true`: the agent receives no MCP server, so the MCP step fails.
        let tid2 = c
            .call(
                "thread.create",
                json!({"workspace": ws, "provider": "test.m5-plain/agent"}),
            )
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        c.call(
            "events.subscribe",
            json!({"topic": format!("thread:{tid2}"), "after": 0}),
        )
        .await;
        c.call(
            "thread.send",
            json!({"thread": tid2, "input": {"text": "no gateway"}}),
        )
        .await;
        let ended = c.events_until("turn.ended").await;
        assert_eq!(ended.last().unwrap()["data"]["outcome"], "failed");

        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn schedules() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        let manifest = kumikit_test_plugins::CLIENT_MANIFEST
            .replace("workspaces = \"linked\"", "workspaces = \"all\"");
        let wasm = std::fs::read(kumikit_test_plugins::CLIENT_WASM).unwrap();
        install(
            &mut c,
            &kkp(
                &dirs,
                "exit-m5-client",
                &manifest,
                &[("plugin.wasm", &wasm)],
            ),
        )
        .await;

        let ws = c
            .call("workspace.open", json!({"path": dirs.workspace()}))
            .await["workspace"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        c.call(
            "command.run",
            json!({
                "plugin": "test.client",
                "command": "timer-set",
                "args": {
                    "key": "exit-m5",
                    "cron": "0 8 * * *",
                    "zone": "UTC",
                    "job": {
                        "workspace": ws,
                        "provider": "test.probe/probe",
                        "model": "echo",
                        "text": "scheduled hello"
                    }
                }
            }),
        )
        .await;

        let list = c.call("schedule.list", json!({})).await["schedules"].clone();
        assert_eq!(list[0]["key"], "exit-m5");
        assert!(list[0]["nextRun"].as_str().is_some());

        c.call(
            "events.subscribe",
            json!({"topic": format!("workspace:{ws}"), "after": 0}),
        )
        .await;
        c.call(
            "schedule.run",
            json!({"plugin": "test.client", "key": "exit-m5"}),
        )
        .await;
        let created = c.events_until("thread.created").await;
        let tid = created.last().unwrap()["data"]["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        c.call(
            "events.subscribe",
            json!({"topic": format!("thread:{tid}"), "after": 0}),
        )
        .await;
        let events = c.events_until("turn.ended").await;
        let started = events
            .iter()
            .find(|e| e["kind"] == "turn.started" && e["topic"] == format!("thread:{tid}"))
            .expect("turn.started");
        assert_eq!(started["data"]["from"], json!({"plugin": "test.client"}));

        c.call(
            "schedule.setEnabled",
            json!({"plugin": "test.client", "key": "exit-m5", "enabled": false}),
        )
        .await;
        assert_eq!(
            c.call("schedule.list", json!({})).await["schedules"][0]["enabled"],
            false
        );
        c.call(
            "schedule.delete",
            json!({"plugin": "test.client", "key": "exit-m5"}),
        )
        .await;
        assert_eq!(
            c.call("schedule.list", json!({})).await["schedules"],
            json!([])
        );

        core.stop().await;
    })
    .await;
}
