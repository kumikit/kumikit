//! One client connection: newline-delimited JSON-RPC 2.0 over any byte stream
//! (`docs/spec/02-protocol.md`). Transport-independent, so tests drive it in memory.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use kumikit_kernel::{Kernel, KernelError, ThreadSubscription};
use kumikit_plugin_host::lifecycle::PluginHost;
use kumikit_protocol::WorkspaceEvent;
use kumikit_protocol::event::{EventNotification, ThreadChange};
use kumikit_protocol::jsonrpc::{Notification, Request, RequestId, Response};
use kumikit_protocol::methods::{
    self as m, CoreInfo, Empty, HelloResult, Method, ModelInfo, NotificationMethod,
    PluginListResult, PluginResult, PluginsChanged, ProviderListResult, ProviderModelsResult,
    SubscribeResult, ThreadCreateResult, ThreadListResult, WorkspaceListResult,
    WorkspaceOpenResult,
};
use kumikit_protocol::model::WorkspaceId;
use kumikit_protocol::{ErrorCode, PROTOCOL_VERSION, ProtocolVersion, RpcError, Topic};
use serde_json::Value;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter,
};
use tokio::sync::{Notify, broadcast, mpsc, watch};
use tokio::task::JoinHandle;

/// Longest accepted message, in bytes (`docs/spec/02-protocol.md#transports`).
pub const MAX_MESSAGE: usize = 8 * 1024 * 1024;

/// Outbound messages buffered per connection before subscriptions wait for the client.
const OUTBOUND: usize = 256;

/// Capability strings this core reports in `core.hello`.
pub const CAPABILITIES: &[&str] = &[
    "threads",
    "approvals",
    "plugins",
    "providers",
    "workspace_events",
];

/// What `core.hello` says about this core.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    /// Core version.
    pub version: String,
    /// Display name, usually the host name.
    pub name: String,
    /// `linux`, `macos` or `windows`.
    pub os: String,
    /// Notified when a client asks the core to stop (`core.shutdown`), after the reply
    /// has been written.
    pub shutdown: Arc<Notify>,
}

impl ServerInfo {
    /// Info for this process and machine.
    pub fn current() -> Self {
        ServerInfo {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            name: crate::paths::host_name(),
            os: std::env::consts::OS.to_owned(),
            shutdown: Arc::new(Notify::new()),
        }
    }
}

/// Serves one connection until the client disconnects, the stream fails, or a message
/// is too long.
pub async fn serve_connection<S>(host: PluginHost, info: Arc<ServerInfo>, stream: S)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (read, write) = tokio::io::split(stream);
    let (tx, rx) = mpsc::channel::<String>(OUTBOUND);
    let writer = tokio::spawn(write_loop(write, rx));
    let mut conn = Conn {
        kernel: host.manager().kernel().clone(),
        host,
        info,
        tx,
        hello: false,
        stop: false,
        subs: HashMap::new(),
        deferred: None,
        plugins_watch: None,
        plugin_changes: None,
    };
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
    loop {
        match read_line(&mut reader, &mut buf, MAX_MESSAGE).await {
            Ok(Line::Data) => {}
            Ok(Line::Eof) => break,
            Ok(Line::TooLong) => {
                let err = RpcError::new(ErrorCode::InvalidRequest, "message too long");
                let _ = conn.tx.send(encode(&Response::error(None, err))).await;
                break;
            }
            Err(e) => {
                tracing::debug!(error = %e, "connection read failed");
                break;
            }
        }
        let line = trim_line(&buf);
        if line.is_empty() {
            continue;
        }
        if let Some(response) = conn.handle(line).await
            && conn.tx.send(response).await.is_err()
        {
            break;
        }
        // A subscription starts only after its result is queued, so the client sees the
        // result before any event of the topic.
        if let Some((topic, sub)) = conn.deferred.take() {
            conn.start(topic, sub);
        }
        if conn.stop {
            break;
        }
        // Likewise, plugin changes are announced only after the hello result.
        if let Some(changes) = conn.plugin_changes.take() {
            conn.plugins_watch = Some(tokio::spawn(announce_plugin_changes(
                conn.host.clone(),
                changes,
                conn.tx.clone(),
            )));
        }
    }
    for (_, task) in conn.subs.drain() {
        task.abort();
    }
    if let Some(task) = conn.plugins_watch.take() {
        task.abort();
    }
    let stop = conn.stop;
    let info = Arc::clone(&conn.info);
    drop(conn);
    let _ = writer.await;
    // Only now: the `core.shutdown` reply has been written.
    if stop {
        info.shutdown.notify_one();
    }
}

/// A subscription whose result is queued but whose events must wait for it.
enum Deferred {
    Thread(ThreadSubscription),
    Workspace(WorkspaceId, broadcast::Receiver<WorkspaceEvent>),
}

struct Conn {
    kernel: Kernel,
    host: PluginHost,
    info: Arc<ServerInfo>,
    tx: mpsc::Sender<String>,
    hello: bool,
    /// Set by `core.shutdown`: close this connection, then stop the core.
    stop: bool,
    subs: HashMap<String, JoinHandle<()>>,
    deferred: Option<(String, Deferred)>,
    /// Sends `core.pluginsChanged`; started after `core.hello`.
    plugins_watch: Option<JoinHandle<()>>,
    /// Taken in `core.hello` before its plugin list is built, so no change between the
    /// two can be missed; handed to `plugins_watch` once the result is queued.
    plugin_changes: Option<watch::Receiver<u64>>,
}

/// Sends `core.pluginsChanged` whenever the enabled plugins change, until the
/// connection closes.
async fn announce_plugin_changes(
    host: PluginHost,
    mut changes: watch::Receiver<u64>,
    tx: mpsc::Sender<String>,
) {
    while changes.changed().await.is_ok() {
        let params = PluginsChanged {
            plugins: crate::plugins::infos(&host),
        };
        let params = match serde_json::to_value(params) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "failed to encode core.pluginsChanged");
                continue;
            }
        };
        let note = Notification::new(m::CorePluginsChanged::NAME, params);
        if tx.send(encode(&note)).await.is_err() {
            return;
        }
    }
}

fn encode<T: serde::Serialize>(value: &T) -> String {
    // Protocol types serialize infallibly (no maps with non-string keys, no custom
    // serializers that fail); keep the connection alive with an error line if one ever
    // does.
    serde_json::to_string(value).unwrap_or_else(|e| {
        tracing::error!(error = %e, "failed to encode a message");
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32000,"message":"encoding failed","data":{"code":"internal"}}}"#.to_owned()
    })
}

fn kernel_err(e: KernelError) -> RpcError {
    if e.code() == ErrorCode::Internal {
        tracing::error!(error = %e, "request failed");
    }
    RpcError::new(e.code(), e.to_string())
}

fn parse<M: Method>(params: Option<Value>) -> Result<M::Params, RpcError> {
    let value = params.unwrap_or_else(|| Value::Object(Default::default()));
    serde_json::from_value(value)
        .map_err(|e| RpcError::new(ErrorCode::InvalidParams, e.to_string()))
}

fn reply<M: Method>(result: M::Result) -> Result<Value, RpcError> {
    serde_json::to_value(result).map_err(|e| RpcError::new(ErrorCode::Internal, e.to_string()))
}

impl Conn {
    /// Handles one line; returns the response to send, if any.
    async fn handle(&mut self, line: &[u8]) -> Option<String> {
        let value: Value = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(e) => {
                let err = RpcError::new(ErrorCode::ParseError, e.to_string());
                return Some(encode(&Response::error(None, err)));
            }
        };
        if value.is_array() {
            let err = RpcError::new(ErrorCode::InvalidRequest, "batches are not supported");
            return Some(encode(&Response::error(None, err)));
        }
        let id: Option<RequestId> = value
            .get("id")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let request: Request = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => {
                let err = RpcError::new(ErrorCode::InvalidRequest, e.to_string());
                return Some(encode(&Response::error(id, err)));
            }
        };
        // Client notifications have no id and get no response; none are defined yet.
        let id = request.id?;
        let response = match self.dispatch(&request.method, request.params).await {
            Ok(result) => Response::success(id, result),
            Err(err) => Response::error(Some(id), err),
        };
        Some(encode(&response))
    }

    async fn dispatch(&mut self, method: &str, params: Option<Value>) -> Result<Value, RpcError> {
        if !self.hello && method != m::CoreHello::NAME {
            return Err(RpcError::new(
                ErrorCode::HandshakeRequired,
                "call core.hello first",
            ));
        }
        let k = self.kernel.clone();
        match method {
            m::CoreHello::NAME => {
                let p = parse::<m::CoreHello>(params)?;
                let theirs = ProtocolVersion::parse(&p.protocol).ok_or_else(|| {
                    RpcError::new(ErrorCode::InvalidParams, "protocol must be MAJOR.MINOR")
                })?;
                if !PROTOCOL_VERSION.compatible_with(theirs) {
                    return Err(RpcError::new(
                        ErrorCode::ProtocolMismatch,
                        format!("core speaks protocol {PROTOCOL_VERSION}"),
                    ));
                }
                if !self.hello {
                    let mut changes = self.host.changes();
                    changes.mark_unchanged();
                    self.plugin_changes = Some(changes);
                }
                self.hello = true;
                tracing::debug!(client = %p.client.name, version = %p.client.version, "hello");
                reply::<m::CoreHello>(HelloResult {
                    protocol: PROTOCOL_VERSION.to_string(),
                    core: CoreInfo {
                        version: self.info.version.clone(),
                        name: self.info.name.clone(),
                        os: self.info.os.clone(),
                    },
                    plugins: crate::plugins::infos(&self.host),
                    capabilities: CAPABILITIES.iter().map(|s| (*s).to_owned()).collect(),
                })
            }
            m::CoreShutdown::NAME => {
                parse::<m::CoreShutdown>(params)?;
                tracing::info!("shutdown requested by a client");
                self.stop = true;
                reply::<m::CoreShutdown>(Empty {})
            }
            m::ProviderList::NAME => {
                parse::<m::ProviderList>(params)?;
                reply::<m::ProviderList>(ProviderListResult {
                    providers: k.provider_list(),
                })
            }
            m::ProviderModels::NAME => {
                let p = parse::<m::ProviderModels>(params)?;
                let models = k.provider_models(&p.provider).await.map_err(kernel_err)?;
                reply::<m::ProviderModels>(ProviderModelsResult {
                    models: models
                        .into_iter()
                        .map(|m| ModelInfo {
                            id: m.id,
                            label: m.label,
                            capabilities: m.capabilities,
                        })
                        .collect(),
                })
            }
            m::WorkspaceList::NAME => {
                parse::<m::WorkspaceList>(params)?;
                let workspaces = k.workspace_list().await.map_err(kernel_err)?;
                reply::<m::WorkspaceList>(WorkspaceListResult { workspaces })
            }
            m::WorkspaceOpen::NAME => {
                let p = parse::<m::WorkspaceOpen>(params)?;
                let workspace = k.workspace_open(&p.path).await.map_err(kernel_err)?;
                reply::<m::WorkspaceOpen>(WorkspaceOpenResult { workspace })
            }
            m::WorkspaceClose::NAME => {
                let p = parse::<m::WorkspaceClose>(params)?;
                k.workspace_close(&p.workspace).await.map_err(kernel_err)?;
                reply::<m::WorkspaceClose>(Empty {})
            }
            m::ThreadList::NAME => {
                let p = parse::<m::ThreadList>(params)?;
                let threads = k
                    .thread_list(&p.workspace, p.include_archived.unwrap_or(false))
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadList>(ThreadListResult { threads })
            }
            m::ThreadCreate::NAME => {
                let p = parse::<m::ThreadCreate>(params)?;
                let thread = k
                    .thread_create(&p.workspace, p.provider, p.model, p.title)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadCreate>(ThreadCreateResult { thread })
            }
            m::ThreadGet::NAME => {
                let p = parse::<m::ThreadGet>(params)?;
                let snapshot = k.thread_get(&p.thread).await.map_err(kernel_err)?;
                reply::<m::ThreadGet>(snapshot)
            }
            m::ThreadSend::NAME => {
                let p = parse::<m::ThreadSend>(params)?;
                let sent = k
                    .thread_send(&p.thread, p.input)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadSend>(sent)
            }
            m::ThreadCancel::NAME => {
                let p = parse::<m::ThreadCancel>(params)?;
                k.thread_cancel(&p.thread).await.map_err(kernel_err)?;
                reply::<m::ThreadCancel>(Empty {})
            }
            m::ThreadRespondApproval::NAME => {
                let p = parse::<m::ThreadRespondApproval>(params)?;
                // Local connections have no device id.
                k.respond_approval(&p.approval, p.decision, None)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadRespondApproval>(Empty {})
            }
            m::ThreadArchive::NAME => {
                let p = parse::<m::ThreadArchive>(params)?;
                k.thread_archive(&p.thread).await.map_err(kernel_err)?;
                reply::<m::ThreadArchive>(Empty {})
            }
            m::EventsSubscribe::NAME => {
                let p = parse::<m::EventsSubscribe>(params)?;
                let topic = Topic::parse(&p.topic).ok_or_else(|| {
                    RpcError::new(
                        ErrorCode::InvalidParams,
                        format!("unknown topic {}", p.topic),
                    )
                })?;
                let (deferred, head) = match &topic {
                    Topic::Thread(t) => {
                        let sub = k.subscribe(t, p.after).await.map_err(kernel_err)?;
                        let head = sub.head();
                        (Deferred::Thread(sub), head)
                    }
                    Topic::Workspace(w) => {
                        if p.after != 0 {
                            return Err(RpcError::new(
                                ErrorCode::InvalidParams,
                                "workspace topics have no log; subscribe with after: 0",
                            ));
                        }
                        // Listen before checking, so no change after the result is missed.
                        let rx = k.workspace_changes();
                        k.workspace_require(w).await.map_err(kernel_err)?;
                        (Deferred::Workspace(w.clone(), rx), 0)
                    }
                    Topic::Reserved(name) => {
                        return Err(RpcError::new(
                            ErrorCode::Unsupported,
                            format!("topic {name} is reserved"),
                        ));
                    }
                };
                let key = topic.to_string();
                if let Some(old) = self.subs.remove(&key) {
                    old.abort();
                }
                self.deferred = Some((key, deferred));
                reply::<m::EventsSubscribe>(SubscribeResult { head })
            }
            m::EventsUnsubscribe::NAME => {
                let p = parse::<m::EventsUnsubscribe>(params)?;
                if let Some(task) = self.subs.remove(&p.topic) {
                    task.abort();
                }
                reply::<m::EventsUnsubscribe>(Empty {})
            }
            m::PluginList::NAME => {
                parse::<m::PluginList>(params)?;
                reply::<m::PluginList>(PluginListResult {
                    plugins: crate::plugins::list(&self.host),
                })
            }
            m::PluginInstall::NAME => {
                let p = parse::<m::PluginInstall>(params)?;
                let plugin = crate::plugins::install(&self.host, &p.path).await?;
                reply::<m::PluginInstall>(PluginResult { plugin })
            }
            m::PluginEnable::NAME => {
                let p = parse::<m::PluginEnable>(params)?;
                let plugin = crate::plugins::enable(&self.host, &p.plugin, &p.grant).await?;
                reply::<m::PluginEnable>(PluginResult { plugin })
            }
            m::PluginDisable::NAME => {
                let p = parse::<m::PluginDisable>(params)?;
                let plugin = crate::plugins::disable(&self.host, &p.plugin).await?;
                reply::<m::PluginDisable>(PluginResult { plugin })
            }
            m::PluginUninstall::NAME => {
                let p = parse::<m::PluginUninstall>(params)?;
                self.host
                    .uninstall(&p.plugin, p.delete_data.unwrap_or(false))
                    .await
                    .map_err(crate::plugins::host_err)?;
                reply::<m::PluginUninstall>(Empty {})
            }
            other => Err(RpcError::new(
                ErrorCode::MethodNotFound,
                format!("no method {other}"),
            )),
        }
    }

    fn start(&mut self, topic: String, deferred: Deferred) {
        let task = match deferred {
            Deferred::Thread(sub) => tokio::spawn(pump(sub, topic.clone(), self.tx.clone())),
            Deferred::Workspace(ws, rx) => tokio::spawn(pump_workspace(
                self.kernel.clone(),
                ws,
                rx,
                topic.clone(),
                self.tx.clone(),
            )),
        };
        self.subs.insert(topic, task);
    }
}

/// Forwards a subscription's events to the connection. Waits when the client is slow:
/// the log is the buffer, so nothing is dropped.
async fn pump(mut sub: ThreadSubscription, topic: String, tx: mpsc::Sender<String>) {
    loop {
        let batch = match sub.next_batch().await {
            Ok(Some(batch)) => batch,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(%topic, error = %e, "subscription failed");
                return;
            }
        };
        for e in batch {
            let params = EventNotification {
                topic: topic.clone(),
                seq: e.seq,
                time: e.time,
                event: e.event.into(),
            };
            let params = match serde_json::to_value(params) {
                Ok(v) => v,
                Err(err) => {
                    tracing::error!(%topic, error = %err, "failed to encode an event");
                    return;
                }
            };
            let line = encode(&Notification::new(m::EventsEvent::NAME, params));
            if tx.send(line).await.is_err() {
                return;
            }
        }
    }
}

/// Forwards live changes to one workspace's threads. Nothing is logged: `seq` counts
/// within this subscription. A listener that lags re-sends every thread of the workspace
/// as `thread.updated`, so the client converges without a gap.
async fn pump_workspace(
    kernel: Kernel,
    workspace: WorkspaceId,
    mut rx: broadcast::Receiver<WorkspaceEvent>,
    topic: String,
    tx: mpsc::Sender<String>,
) {
    let mut seq = 0u64;
    loop {
        let events = match rx.recv().await {
            Ok(event) if event.thread().workspace == workspace => vec![event],
            Ok(_) => continue,
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                tracing::debug!(%topic, missed, "workspace listener lagged; resending threads");
                match kernel.thread_list(&workspace, true).await {
                    Ok(threads) => threads
                        .into_iter()
                        .map(|thread| WorkspaceEvent::ThreadUpdated(ThreadChange { thread }))
                        .collect(),
                    Err(e) => {
                        tracing::warn!(%topic, error = %e, "cannot resend threads");
                        return;
                    }
                }
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        for event in events {
            seq += 1;
            let params = EventNotification {
                topic: topic.clone(),
                seq,
                time: kernel.now(),
                event: event.into(),
            };
            let params = match serde_json::to_value(params) {
                Ok(v) => v,
                Err(err) => {
                    tracing::error!(%topic, error = %err, "failed to encode an event");
                    return;
                }
            };
            if tx
                .send(encode(&Notification::new(m::EventsEvent::NAME, params)))
                .await
                .is_err()
            {
                return;
            }
        }
    }
}

async fn write_loop<W: AsyncWrite>(write: W, mut rx: mpsc::Receiver<String>) {
    let out = BufWriter::new(write);
    tokio::pin!(out);
    while let Some(line) = rx.recv().await {
        let mut ok = write_one(&mut out, &line).await;
        // Write whatever else is ready before flushing once.
        while ok {
            match rx.try_recv() {
                Ok(next) => ok = write_one(&mut out, &next).await,
                Err(_) => break,
            }
        }
        if !ok || out.flush().await.is_err() {
            return;
        }
    }
    let _ = out.shutdown().await;
}

async fn write_one<W: AsyncWrite>(out: &mut std::pin::Pin<&mut BufWriter<W>>, line: &str) -> bool {
    out.write_all(line.as_bytes()).await.is_ok() && out.write_all(b"\n").await.is_ok()
}

/// Result of reading one line.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    /// A line (possibly the last one, without a newline) is in the buffer.
    Data,
    /// The stream ended.
    Eof,
    /// The line exceeds the limit; the stream is left mid-line.
    TooLong,
}

async fn read_line<R: AsyncBufRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> io::Result<Line> {
    buf.clear();
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                Line::Eof
            } else {
                Line::Data
            });
        }
        let (chunk, done) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (i, true),
            None => (available.len(), false),
        };
        if buf.len() + chunk > max {
            return Ok(Line::TooLong);
        }
        buf.extend_from_slice(&available[..chunk]);
        r.consume(if done { chunk + 1 } else { chunk });
        if done {
            return Ok(Line::Data);
        }
    }
}

fn trim_line(buf: &[u8]) -> &[u8] {
    let end = buf
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    let start = buf
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(end);
    &buf[start..end]
}

#[cfg(test)]
mod tests;
