use std::sync::Arc;

use kumikit_kernel::{Kernel, KernelConfig};
use kumikit_protocol::methods::METHOD_NAMES;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};

use kumikit_kernel::provider::{
    BoxFuture, EventSink, Model, Provider, ProviderEvent, ProviderResult, SessionConfig,
};
use kumikit_plugin_host::engine::EngineConfig;
use kumikit_plugin_host::lifecycle::{HostConfig, PluginHost};
use kumikit_plugin_host::manager::{Dirs, Manager};
use kumikit_plugin_host::runtime::Runtime;
use kumikit_protocol::model::{ApprovalId, Decision, UserInput};

use super::*;

/// A provider for these tests: echoes the input word by word. Lives in the test module;
/// nothing provider-shaped is compiled into the core.
/// The input `ask` requests an approval and waits for its answer.
#[derive(Debug, Default)]
struct Echo {
    answered: Arc<tokio::sync::Notify>,
}

impl Provider for Echo {
    fn id(&self) -> &str {
        "test.echo/echo"
    }
    fn list_models(&self) -> BoxFuture<'_, ProviderResult<Vec<Model>>> {
        Box::pin(async {
            Ok(vec![Model {
                id: "echo".into(),
                label: "Echo".into(),
                capabilities: vec![],
            }])
        })
    }
    fn start_session(&self, c: SessionConfig) -> BoxFuture<'_, ProviderResult<String>> {
        Box::pin(async move { Ok(format!("s-{}", c.thread)) })
    }
    fn send(
        &self,
        _s: String,
        input: UserInput,
        sink: EventSink,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async move {
            let err = |e: KernelError| kumikit_kernel::provider::ProviderError::new(e.to_string());
            if input.text == "ask" {
                sink.request_approval("tool", "run it?", None)
                    .await
                    .map_err(err)?;
                self.answered.notified().await;
            }
            for word in input.text.split_inclusive(' ') {
                sink.emit(ProviderEvent::TextDelta { text: word.into() })
                    .await
                    .map_err(err)?;
            }
            sink.emit(ProviderEvent::TextDone {
                text: input.text.clone(),
            })
            .await
            .map_err(err)?;
            sink.emit(ProviderEvent::Usage {
                input_tokens: 1,
                output_tokens: 1,
                cost: None,
            })
            .await
            .map_err(err)?;
            Ok(())
        })
    }
    fn respond_approval(
        &self,
        _s: String,
        _a: ApprovalId,
        _d: Decision,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        self.answered.notify_one();
        Box::pin(async { Ok(()) })
    }
    fn cancel(&self, _s: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn end_session(&self, _s: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

struct Client {
    _dir: tempfile::TempDir,
    write: tokio::io::WriteHalf<DuplexStream>,
    lines: Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
    next_id: i64,
}

async fn client() -> Client {
    client_as(Peer::Local).await
}

async fn client_as(peer: Peer) -> Client {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::open(KernelConfig::new(dir.path().join("db")))
        .await
        .unwrap();
    kernel.register_provider(Arc::new(Echo::default())).unwrap();
    let dirs = Dirs::new(dir.path());
    let runtime = Arc::new(Runtime::new(EngineConfig::default(), dirs.cache.clone()));
    let manager = Manager::open(kernel, dirs, runtime).await.unwrap();
    let host = PluginHost::open(manager, HostConfig::default()).await;
    let (ours, theirs) = tokio::io::duplex(64 * 1024);
    let info = Arc::new(ServerInfo {
        version: "0.0.0".into(),
        name: "test".into(),
        os: "test".into(),
        shutdown: Arc::new(tokio::sync::Notify::new()),
        remote: None,
    });
    let (read, write) = tokio::io::split(theirs);
    tokio::spawn(serve_messages(
        host,
        info,
        LineReader::new(read),
        LineWriter::new(write),
        peer,
        std::future::pending(),
    ));
    let (read, write) = tokio::io::split(ours);
    Client {
        _dir: dir,
        write,
        lines: BufReader::new(read).lines(),
        next_id: 0,
    }
}

impl Client {
    async fn raw(&mut self, line: &str) {
        self.write.write_all(line.as_bytes()).await.unwrap();
        self.write.write_all(b"\n").await.unwrap();
    }

    async fn recv(&mut self) -> Option<Value> {
        let line = self.lines.next_line().await.unwrap()?;
        Some(serde_json::from_str(&line).unwrap())
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.raw(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string())
            .await;
        loop {
            let msg = self.recv().await.expect("connection closed");
            if msg["id"] == id {
                return msg;
            }
        }
    }

    async fn hello(&mut self) {
        let r = self
            .call(
                "core.hello",
                json!({"protocol":"0.4","client":{"name":"t","version":"1"}}),
            )
            .await;
        assert_eq!(r["result"]["protocol"], "0.4", "{r}");
    }

    fn dir(&self) -> String {
        self._dir.path().to_str().unwrap().to_owned()
    }
}

fn code(r: &Value) -> &str {
    r["error"]["data"]["code"].as_str().unwrap_or("")
}

#[tokio::test]
async fn parse_error_keeps_the_connection() {
    let mut c = client().await;
    c.raw("{not json").await;
    let r = c.recv().await.unwrap();
    assert_eq!(r["error"]["code"], -32700);
    assert_eq!(r["id"], Value::Null);
    c.hello().await;
}

#[tokio::test]
async fn batches_are_rejected() {
    let mut c = client().await;
    c.raw(r#"[{"jsonrpc":"2.0","id":1,"method":"core.hello"}]"#)
        .await;
    let r = c.recv().await.unwrap();
    assert_eq!(code(&r), "invalid_request");
}

#[tokio::test]
async fn invalid_request_keeps_its_id() {
    let mut c = client().await;
    c.raw(r#"{"jsonrpc":"1.0","id":9,"method":"core.hello"}"#)
        .await;
    let r = c.recv().await.unwrap();
    assert_eq!(code(&r), "invalid_request");
    assert_eq!(r["id"], 9);
}

#[tokio::test]
async fn handshake_first() {
    let mut c = client().await;
    let r = c.call("workspace.list", json!({})).await;
    assert_eq!(code(&r), "handshake_required");
    let r = c
        .call(
            "core.hello",
            json!({"protocol":"1.0","client":{"name":"t","version":"1"}}),
        )
        .await;
    assert_eq!(code(&r), "protocol_mismatch");
    let r = c
        .call(
            "core.hello",
            json!({"protocol":"zero","client":{"name":"t","version":"1"}}),
        )
        .await;
    assert_eq!(code(&r), "invalid_params");
    c.hello().await;
    let r = c.call("workspace.list", json!({})).await;
    assert_eq!(r["result"]["workspaces"], json!([]));
}

#[tokio::test]
async fn hello_describes_the_core() {
    let mut c = client().await;
    let r = c
        .call(
            "core.hello",
            json!({"protocol":"0.4","client":{"name":"t","version":"1"}}),
        )
        .await;
    assert_eq!(
        r["result"],
        json!({"protocol":"0.4","core":{"version":"0.0.0","name":"test","os":"test"},
               "plugins":[],"capabilities":["threads","approvals","plugins","providers","workspace_events"]})
    );
}

#[tokio::test]
async fn every_method_is_routed() {
    let mut c = client().await;
    c.hello().await;
    // `core.shutdown` closes the connection; it has its own test.
    for method in METHOD_NAMES.iter().filter(|m| **m != "core.shutdown") {
        let r = c.call(method, json!({})).await;
        assert_ne!(code(&r), "method_not_found", "{method} is not dispatched");
    }
    let r = c.call("nope.nothing", json!({})).await;
    assert_eq!(code(&r), "method_not_found");
    assert_eq!(r["error"]["code"], -32601);
}

#[tokio::test]
async fn bad_params_are_invalid_params() {
    let mut c = client().await;
    c.hello().await;
    let r = c.call("thread.get", json!({"thread": 5})).await;
    assert_eq!(code(&r), "invalid_params");
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn client_notifications_get_no_response() {
    let mut c = client().await;
    c.hello().await;
    c.raw(r#"{"jsonrpc":"2.0","method":"workspace.list"}"#)
        .await;
    // The next response is for the next call, not for the notification.
    let r = c.call("workspace.list", json!({})).await;
    assert_eq!(r["id"], c.next_id);
}

#[tokio::test]
async fn oversized_line_closes_the_connection() {
    let mut c = client().await;
    let big = "x".repeat(MAX_MESSAGE + 1);
    // Write in the background: the server stops reading once the limit is hit.
    let mut write = c.write;
    tokio::spawn(async move {
        let _ = write.write_all(big.as_bytes()).await;
    });
    let r = c.lines.next_line().await.unwrap().unwrap();
    let r: Value = serde_json::from_str(&r).unwrap();
    assert_eq!(code(&r), "invalid_request");
    assert!(
        c.lines.next_line().await.unwrap().is_none(),
        "connection should close"
    );
}

#[tokio::test]
async fn topics() {
    let mut c = client().await;
    c.hello().await;
    let r = c
        .call("events.subscribe", json!({"topic":"core","after":0}))
        .await;
    assert_eq!(code(&r), "unsupported");
    let r = c
        .call("events.subscribe", json!({"topic":"bogus","after":0}))
        .await;
    assert_eq!(code(&r), "invalid_params");
    let r = c
        .call("events.subscribe", json!({"topic":"thread:none","after":0}))
        .await;
    assert_eq!(code(&r), "not_found");
}

#[tokio::test]
async fn subscribe_result_comes_before_events() {
    let mut c = client().await;
    c.hello().await;
    let dir = c.dir();
    let ws = c.call("workspace.open", json!({"path": dir})).await;
    let ws = ws["result"]["workspace"]["id"].clone();
    let t = c
        .call("thread.create", json!({"workspace": ws, "model": "echo"}))
        .await;
    let thread = t["result"]["thread"]["id"].as_str().unwrap().to_owned();
    assert_eq!(t["result"]["thread"]["provider"], "test.echo/echo");
    let topic = format!("thread:{thread}");
    // Produce the whole turn first, so replay has events ready immediately.
    c.call(
        "thread.send",
        json!({"thread": thread, "input": {"text": "a b"}}),
    )
    .await;
    let first = c
        .call("events.subscribe", json!({"topic": topic, "after": 0}))
        .await;
    let mut seqs = vec![];
    while seqs.last() != Some(&6) {
        let n = c.recv().await.unwrap();
        assert_eq!(n["method"], "events.event");
        assert_eq!(n["params"]["topic"], topic);
        seqs.push(n["params"]["seq"].as_u64().unwrap());
    }
    assert_eq!(seqs, [1, 2, 3, 4, 5, 6]);
    // The result was the first message for this topic (call() skips nothing but
    // responses with other ids, and no event arrived before it).
    assert!(first["result"]["head"].as_u64().unwrap() <= 6);

    // Resubscribing replaces the old subscription; unsubscribing stops it.
    let r = c
        .call("events.subscribe", json!({"topic": topic, "after": 6}))
        .await;
    assert_eq!(r["result"]["head"], 6);
    let r = c.call("events.unsubscribe", json!({"topic": topic})).await;
    assert_eq!(r["result"], json!({}));
}

#[tokio::test]
async fn auth_on_a_local_connection() {
    let mut c = client().await;
    c.hello().await;
    for method in ["auth.pair", "auth.token"] {
        let r = c.call(method, json!({})).await;
        assert_eq!(code(&r), "unsupported", "{method}");
    }
    // Remote access is off in this core.
    let r = c.call("auth.pairing.create", json!({})).await;
    assert_eq!(code(&r), "unsupported");
    let r = c.call("auth.devices.list", json!({})).await;
    assert_eq!(r["result"], json!({"devices": []}), "{r}");
    let r = c
        .call("auth.devices.revoke", json!({"device": "nope"}))
        .await;
    assert_eq!(code(&r), "not_found");
}

#[tokio::test]
async fn approvals_record_the_answering_device() {
    for (peer, by) in [
        (
            Peer::Device("01JCDV00000000000000000001".into()),
            Some("01JCDV00000000000000000001"),
        ),
        (Peer::Local, None),
    ] {
        let mut c = client_as(peer).await;
        c.hello().await;
        let dir = tempfile::tempdir().unwrap();
        let ws = c
            .call(
                "workspace.open",
                json!({"path": dir.path().to_str().unwrap()}),
            )
            .await;
        let t = c
            .call(
                "thread.create",
                json!({"workspace": ws["result"]["workspace"]["id"]}),
            )
            .await;
        let thread = t["result"]["thread"]["id"].clone();
        c.call(
            "events.subscribe",
            json!({"topic": format!("thread:{}", thread.as_str().unwrap()), "after": 0}),
        )
        .await;
        c.call(
            "thread.send",
            json!({"thread": thread, "input": {"text": "ask"}}),
        )
        .await;
        let approval = loop {
            let m = c.recv().await.unwrap();
            if m["params"]["kind"] == "approval.requested" {
                break m["params"]["data"]["approval"]["id"].clone();
            }
        };
        c.call(
            "thread.respondApproval",
            json!({"approval": approval, "decision": "allow"}),
        )
        .await;
        let resolved = loop {
            let m = c.recv().await.unwrap();
            if m["params"]["kind"] == "approval.resolved" {
                break m["params"]["data"].clone();
            }
        };
        assert_eq!(resolved["by"].as_str(), by, "{resolved}");
    }
}
