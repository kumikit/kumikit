//! The tool gateway against the real binary (`docs/spec/03-kernel.md#tool-gateway`): a
//! provider session granted `tools` gets an `mcp-server` record; the agent (here, the
//! test) starts `kumikit mcp` with it and calls plugin tools over MCP during a turn. Write
//! tools wait for an approval; tokens die with their session; no grant, no gateway.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{Value, json};
use support::{Client, Core, DEADLINE, Dirs, PROBE_MANIFEST, install, kkp};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

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

fn probe(dirs: &Dirs, tools: bool) -> PathBuf {
    let manifest = if tools {
        PROBE_MANIFEST.replace(
            "spawn = [\"fake-acp-agent\"]",
            "spawn = [\"fake-acp-agent\"]\ntools = true",
        )
    } else {
        PROBE_MANIFEST.to_owned()
    };
    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    kkp(
        dirs,
        if tools { "probe-tools" } else { "probe-plain" },
        &manifest,
        &[("plugin.wasm", &wasm)],
    )
}

fn tool_plugin(dirs: &Dirs) -> PathBuf {
    let src = Path::new(kumikit_test_plugins::TOOL_DIR);
    let manifest = std::fs::read_to_string(src.join("plugin.toml")).unwrap();
    let wasm = std::fs::read(kumikit_test_plugins::TOOL_WASM).unwrap();
    let echo = std::fs::read(src.join("tools/echo.json")).unwrap();
    let empty = std::fs::read(src.join("tools/empty.json")).unwrap();
    std::fs::create_dir_all(dirs.root().join("tool-src/tools")).unwrap();
    kkp(
        dirs,
        "tool",
        &manifest,
        &[
            ("plugin.wasm", &wasm),
            ("tools/echo.json", &echo),
            ("tools/empty.json", &empty),
        ],
    )
}

/// The agent's side: `kumikit mcp` started as the record says.
struct Agent {
    child: Child,
    stdin: ChildStdin,
    stdout: tokio::io::Lines<BufReader<ChildStdout>>,
    next: u64,
}

impl Agent {
    fn start(record: &Value) -> Agent {
        let mut cmd = Command::new(record["command"].as_str().unwrap());
        for a in record["args"].as_array().unwrap() {
            cmd.arg(a.as_str().unwrap());
        }
        for (k, v) in record["env"].as_object().unwrap() {
            cmd.env(k, v.as_str().unwrap());
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        Agent {
            child,
            stdin,
            stdout,
            next: 1,
        }
    }

    /// Sends a request and returns its response, skipping notifications.
    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        loop {
            let line = self
                .stdout
                .next_line()
                .await
                .unwrap()
                .expect("kumikit mcp ended");
            let v: Value = serde_json::from_str(&line).unwrap();
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    async fn tool(&mut self, name: &str, args: Value) -> Value {
        self.call("tools/call", json!({ "name": name, "arguments": args }))
            .await["result"]
            .clone()
    }
}

/// Starts a thread on the probe and returns it with its `mcp-server` record (or null).
async fn session(c: &mut Client, ws: &Value) -> (String, Value) {
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
    c.call(
        "thread.send",
        json!({ "thread": thread, "input": { "text": format!("mcp-info {thread}") } }),
    )
    .await;
    let events = c.events_until("turn.ended").await;
    let done = events.iter().find(|e| e["kind"] == "text.done").unwrap();
    let record: Value = serde_json::from_str(done["data"]["text"].as_str().unwrap()).unwrap();
    (thread, record)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_calls_plugin_tools_through_the_gateway() {
    within(async {
        let dirs = Dirs::new();
        let (_core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe(&dirs, true)).await;
        install(&mut c, &tool_plugin(&dirs)).await;
        let ws = c.call("workspace.open", json!({ "path": dirs.workspace() })).await["workspace"].clone();
        let (thread, record) = session(&mut c, &ws["id"]).await;
        assert_eq!(record["name"], "kumikit");
        assert_eq!(record["args"][0], "mcp");
        assert!(record["env"]["KUMIKIT_MCP_TOKEN"].as_str().unwrap().len() >= 64);

        let mut agent = Agent::start(&record);
        let init = agent.call("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } })).await;
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "kumikit");
        let list = agent.call("tools/list", json!({})).await;
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["test_tool__echo", "test_tool__cwd", "test_tool__touch"]);

        // Outside a turn nothing runs.
        let r = agent.tool("test_tool__echo", json!({ "text": "hi" })).await;
        assert_eq!(r["isError"], true, "{r}");

        // During a turn (the probe hangs until cancelled), read tools just run.
        c.call("thread.send", json!({ "thread": thread, "input": { "text": "hang" } })).await;
        c.events_until("turn.started").await;
        let r = agent.tool("test_tool__echo", json!({ "text": "hi" })).await;
        assert_eq!(r["content"][0]["text"], "hi", "{r}");
        assert_eq!(r["structuredContent"]["thread"], thread.as_str());
        let r = agent.tool("test_tool__echo", json!({ "nope": 1 })).await;
        assert!(r["content"][0]["text"].as_str().unwrap().contains("invalid arguments"), "{r}");
        let r = agent.tool("test_tool__cwd", json!({})).await;
        assert_eq!(
            std::fs::canonicalize(r["content"][0]["text"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(ws["root"].as_str().unwrap()).unwrap(),
            "the tool acts in the thread's workspace"
        );
        let events = c.events_until("tool.finished").await;
        let started = events.iter().find(|e| e["kind"] == "tool.started").unwrap();
        assert_eq!(started["data"]["source"], json!({ "plugin": "test.tool", "tool": "echo" }));

        // A write tool waits for an approval.
        let pending = tokio::spawn(async move {
            let r = agent.tool("test_tool__touch", json!({ "text": "x" })).await;
            (agent, r)
        });
        let events = c.events_until("approval.requested").await;
        let approval = events.last().unwrap()["data"]["approval"].clone();
        assert_eq!(approval["kind"], "tool");
        assert_eq!(approval["detail"]["tool"], "test_tool__touch");
        c.call("thread.respondApproval", json!({ "approval": approval["id"], "decision": "allow_always" })).await;
        let (mut agent, r) = pending.await.unwrap();
        assert_eq!(r["content"][0]["text"], "touched x", "{r}");
        // `allow_always` covers it in this thread from now on.
        let r = agent.tool("test_tool__touch", json!({ "text": "y" })).await;
        assert_eq!(r["content"][0]["text"], "touched y", "{r}");

        c.call("thread.cancel", json!({ "thread": thread })).await;
        c.events_until("turn.ended").await;
        let r = agent.tool("test_tool__echo", json!({ "text": "late" })).await;
        assert_eq!(r["isError"], true, "{r}");

        // Disabling the provider ends its sessions, and with them the token.
        drop(agent);
        c.call("plugin.disable", json!({ "plugin": "test.probe" })).await;
        let mut stale = Agent::start(&record);
        let out = stale.child.wait().await.unwrap();
        assert!(!out.success(), "a stale token must be refused");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_denied_write_tool_does_not_run() {
    within(async {
        let dirs = Dirs::new();
        let (_core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe(&dirs, true)).await;
        install(&mut c, &tool_plugin(&dirs)).await;
        let ws = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]
            .clone();
        let (thread, record) = session(&mut c, &ws["id"]).await;
        let mut agent = Agent::start(&record);
        agent
            .call(
                "initialize",
                json!({ "protocolVersion": "2025-11-25", "capabilities": {} }),
            )
            .await;
        c.call(
            "thread.send",
            json!({ "thread": thread, "input": { "text": "hang" } }),
        )
        .await;
        c.events_until("turn.started").await;
        let pending =
            tokio::spawn(
                async move { agent.tool("test_tool__touch", json!({ "text": "x" })).await },
            );
        let events = c.events_until("approval.requested").await;
        let approval = events.last().unwrap()["data"]["approval"]["id"].clone();
        c.call(
            "thread.respondApproval",
            json!({ "approval": approval, "decision": "deny" }),
        )
        .await;
        let r = pending.await.unwrap();
        assert_eq!(r["isError"], true, "{r}");
        let events = c.events_until("tool.finished").await;
        assert_eq!(events.last().unwrap()["data"]["ok"], false);
        assert_eq!(
            events.last().unwrap()["data"]["result"],
            json!({ "denied": true })
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_tools_grant_no_gateway_and_bad_tokens_are_refused() {
    within(async {
        let dirs = Dirs::new();
        let (_core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe(&dirs, false)).await;
        let ws = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]
            .clone();
        let (_thread, record) = session(&mut c, &ws["id"]).await;
        assert_eq!(
            record,
            Value::Null,
            "a provider without `tools` gets no gateway"
        );

        let mut bogus = Agent::start(&json!({
            "command": env!("CARGO_BIN_EXE_kumikit"),
            "args": ["mcp", "--socket", dirs.address],
            "env": { "KUMIKIT_MCP_TOKEN": "0".repeat(64) },
        }));
        let out = bogus.child.wait().await.unwrap();
        assert!(!out.success());
        let mut err = String::new();
        use tokio::io::AsyncReadExt;
        bogus
            .child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut err)
            .await
            .unwrap();
        assert!(err.contains("refused the tool gateway"), "{err}");

        // gateway.attach anywhere but first is refused too.
        let r = c.request("gateway.attach", json!({ "token": "x" })).await;
        assert_eq!(r["error"]["data"]["code"], "invalid_request");
    })
    .await;
}

/// The whole chain: provider-acp passes the gateway to its agent (the fake agent, replaying
/// a script with an MCP step), the agent calls a plugin tool, and the answer reaches the
/// transcript, logged with its source.
#[tokio::test(flavor = "multi_thread")]
async fn an_acp_agent_gets_the_gateway_from_its_provider() {
    within(async {
        let dirs = Dirs::new();
        let (_core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &support::provider_acp_kkp(&dirs)).await;
        install(&mut c, &tool_plugin(&dirs)).await;
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/acp-gateway.jsonl")
            .to_string_lossy()
            .replace('\\', "\\\\");
        let manifest = format!(
            "id = \"test.gateway-agent\"\nname = \"Gateway agent\"\nversion = \"0.1.0\"\nkind = [\"provider\"]\napi = \"^0.2\"\n\
             [permissions]\nspawn = [\"fake-acp-agent\"]\ntools = true\n\
             [contributes]\nproviders = [{{ id = \"agent\", name = \"Agent\", acp = {{ command = \"fake-acp-agent\", args = [\"replay\", \"{script}\"] }} }}]\n"
        );
        install(&mut c, &kkp(&dirs, "gateway-agent", &manifest, &[])).await;
        let ws = c.call("workspace.open", json!({ "path": dirs.workspace() })).await["workspace"].clone();
        let thread = c
            .call("thread.create", json!({ "workspace": ws["id"], "provider": "test.gateway-agent/agent" }))
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        c.call("events.subscribe", json!({ "topic": format!("thread:{thread}"), "after": 0 })).await;
        c.call("thread.send", json!({ "thread": thread, "input": { "text": "where are you?" } })).await;
        let events = c.events_until("turn.ended").await;
        assert_eq!(events.last().unwrap()["data"]["outcome"], "done", "{events:?}");
        let started = events.iter().find(|e| e["kind"] == "tool.started").expect("a gateway call");
        assert_eq!(started["data"]["source"], json!({ "plugin": "test.tool", "tool": "cwd" }));
        let said: String = events
            .iter()
            .filter(|e| e["kind"] == "text.delta")
            .map(|e| e["data"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(
            std::fs::canonicalize(said.trim()).unwrap(),
            std::fs::canonicalize(ws["root"].as_str().unwrap()).unwrap(),
            "the agent says what the tool answered: {said}"
        );
    })
    .await;
}
