//! One client connection: newline-delimited JSON-RPC 2.0 over any byte stream
//! (`docs/spec/02-protocol.md`). Transport-independent, so tests drive it in memory.

use std::collections::HashMap;
use std::sync::Arc;

use kumikit_kernel::{Kernel, KernelError, ThreadSubscription};
use kumikit_plugin_host::lifecycle::PluginHost;
use kumikit_protocol::event::{EventNotification, ThreadChange};
use kumikit_protocol::jsonrpc::{Notification, Request, RequestId, Response};
use kumikit_protocol::methods::{
    self as m, CoreInfo, Empty, HelloResult, Method, ModelInfo, NotificationMethod,
    PluginListResult, PluginResult, PluginsChanged, ProviderListResult, ProviderModelsResult,
    SubscribeResult, ThreadCreateResult, ThreadListResult, ThreadUpdateResult, WorkspaceListResult,
    WorkspaceOpenResult,
};
use kumikit_protocol::model::WorkspaceId;
use kumikit_protocol::{ErrorCode, PROTOCOL_VERSION, ProtocolVersion, RpcError, Topic};
use kumikit_protocol::{UsageEvent, WorkspaceEvent};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::messages::{Close, LineReader, LineWriter, MessageReader, MessageWriter, Received};
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
    "secrets",
    "settings",
    "commands",
    "schedules",
    "gateway",
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
    /// Remote access, when the core runs with `--listen`.
    pub remote: Option<Arc<crate::remote::Remote>>,
    /// Plugin registries; set by `serve` from the data directory.
    pub registry: Option<Arc<crate::registry::CoreRegistry>>,
}

/// Who is on the other end of a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peer {
    /// A local transport (socket, pipe, or SSH arriving on them): the core's own user.
    Local,
    /// A paired device on the remote transport, by device id.
    Device(String),
}

impl ServerInfo {
    /// Info for this process and machine.
    pub fn current() -> Self {
        ServerInfo {
            version: kumikit_protocol::APP_VERSION.to_owned(),
            name: crate::paths::host_name(),
            os: std::env::consts::OS.to_owned(),
            shutdown: Arc::new(Notify::new()),
            remote: None,
            registry: None,
        }
    }
}

/// Serves one newline-delimited connection (socket, pipe, `kumikit bridge`) until the
/// client disconnects, the stream fails, or a message is too long.
pub async fn serve_connection<S>(host: PluginHost, info: Arc<ServerInfo>, stream: S)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (read, write) = tokio::io::split(stream);
    serve_messages(
        host,
        info,
        LineReader::new(read),
        LineWriter::new(write),
        Peer::Local,
        std::future::pending(),
    )
    .await;
}

/// Serves one connection over any message transport, until the client disconnects, the
/// transport fails, a message is too long or not text, or `stop` resolves (the device was
/// revoked); then the transport closes with `stop`'s reason. `stop` is polled next to
/// `reader.recv()`, so a reader whose `recv` is not cancel-safe must get a `stop` that
/// never resolves.
pub async fn serve_messages<R, W>(
    host: PluginHost,
    info: Arc<ServerInfo>,
    mut reader: R,
    writer: W,
    peer: Peer,
    stop: impl Future<Output = Close> + Send,
) where
    R: MessageReader,
    W: MessageWriter,
{
    let (tx, rx) = mpsc::channel::<String>(OUTBOUND);
    let (close_tx, close_rx) = tokio::sync::oneshot::channel();
    let writer = tokio::spawn(writer.run(rx, close_rx));
    tokio::pin!(stop);
    let mut close_tx = Some(close_tx);
    let mut conn = Conn {
        kernel: host.manager().kernel().clone(),
        host,
        info,
        tx,
        hello: false,
        device: match peer {
            Peer::Local => None,
            Peer::Device(id) => Some(id),
        },
        stop: false,
        subs: HashMap::new(),
        deferred: None,
        plugins_watch: None,
        plugin_changes: None,
    };
    let mut first = true;
    loop {
        let received = tokio::select! {
            biased;
            close = &mut stop => {
                tracing::debug!(device = ?conn.device, code = close.code, "closing the connection");
                if let Some(tx) = close_tx.take() {
                    let _ = tx.send(close);
                }
                break;
            }
            r = reader.recv() => r,
        };
        let message = match received {
            Ok(Received::Message(m)) => m,
            Ok(Received::Eof) => {
                tracing::debug!(device = ?conn.device, "the client closed the connection");
                break;
            }
            Ok(Received::Refused(why)) => {
                tracing::debug!(device = ?conn.device, why, "closing the connection");
                let err = RpcError::new(ErrorCode::InvalidRequest, why);
                let _ = conn.tx.send(encode(&Response::error(None, err))).await;
                break;
            }
            Err(e) => {
                tracing::debug!(error = %e, "connection read failed");
                break;
            }
        };
        if message.is_empty() {
            continue;
        }
        // A local connection whose first message is `gateway.attach` carries MCP from
        // then on (`docs/spec/03-kernel.md#tool-gateway`).
        if std::mem::take(&mut first)
            && let Some((id, token)) = attach_request(&message)
        {
            let session = match (&conn.device, conn.kernel.gateway_session(&token)) {
                (Some(_), _) => Err(RpcError::new(
                    ErrorCode::PermissionDenied,
                    "the tool gateway is only for local connections",
                )),
                (None, None) => Err(RpcError::new(
                    ErrorCode::Unauthenticated,
                    "this gateway token is not valid (its session ended, or it never existed)",
                )),
                (None, Some(s)) => Ok(s),
            };
            match session {
                Ok(session) => {
                    let ok = Response::success(id, Value::Object(Default::default()));
                    if conn.tx.send(encode(&ok)).await.is_ok() {
                        crate::gateway::serve(conn.host.clone(), session, reader, conn.tx.clone())
                            .await;
                    }
                }
                Err(err) => {
                    tracing::warn!(device = ?conn.device, "gateway.attach refused");
                    let _ = conn.tx.send(encode(&Response::error(Some(id), err))).await;
                }
            }
            break;
        }
        if let Some(response) = conn.handle(&message).await
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
    Usage(broadcast::Receiver<UsageEvent>),
}

struct Conn {
    kernel: Kernel,
    host: PluginHost,
    info: Arc<ServerInfo>,
    tx: mpsc::Sender<String>,
    hello: bool,
    /// The paired device on a remote connection; `None` on local ones.
    device: Option<String>,
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

/// A kernel device record as the protocol shows it; `current` marks the caller's own.
fn device_info(d: kumikit_kernel::devices::DeviceRecord, current: Option<&str>) -> m::Device {
    m::Device {
        current: (current == Some(d.id.as_str())).then_some(true),
        id: d.id,
        name: d.name,
        created: d.created,
        last_seen: d.last_seen,
    }
}

/// The id and token of a `gateway.attach` request; `None` for any other message.
fn attach_request(message: &[u8]) -> Option<(RequestId, String)> {
    let request: Request = serde_json::from_slice(message).ok()?;
    if request.method != m::GatewayAttach::NAME {
        return None;
    }
    let id = request.id?;
    let params: m::GatewayAttachParams = serde_json::from_value(request.params?).ok()?;
    Some((id, params.token))
}

fn check_secret_name(name: &str) -> Result<(), RpcError> {
    if kumikit_plugin_host::manifest::is_secret_name(name) {
        Ok(())
    } else {
        Err(RpcError::new(
            ErrorCode::InvalidParams,
            format!("secret name `{name}` is not valid"),
        ))
    }
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
    /// The core's registries, or `unsupported` where none are configured (unit tests).
    fn registry(&self) -> Result<Arc<crate::registry::CoreRegistry>, RpcError> {
        self.info.registry.clone().ok_or_else(|| {
            RpcError::new(ErrorCode::Unsupported, "this core has no plugin registries")
        })
    }

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
        // Method names and timing only: params and results are never logged here.
        let started = std::time::Instant::now();
        tracing::debug!(device = ?self.device, method = %request.method, "request");
        let response = match self.dispatch(&request.method, request.params).await {
            Ok(result) => Response::success(id, result),
            Err(err) => Response::error(Some(id), err),
        };
        tracing::debug!(
            device = ?self.device,
            method = %request.method,
            ms = started.elapsed().as_millis() as u64,
            "answered"
        );
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
                    capabilities: CAPABILITIES
                        .iter()
                        .copied()
                        .chain(self.info.registry.is_some().then_some("registry"))
                        .chain(self.info.remote.is_some().then_some("devices"))
                        .map(str::to_owned)
                        .collect(),
                })
            }
            m::CoreActivity::NAME => {
                parse::<m::CoreActivity>(params)?;
                let activity = k.activity().await.map_err(kernel_err)?;
                reply::<m::CoreActivity>(activity)
            }
            m::CoreShutdown::NAME => {
                let p = parse::<m::CoreShutdown>(params)?;
                if p.if_idle == Some(true) {
                    let activity = k.activity().await.map_err(kernel_err)?;
                    if !activity.idle {
                        return Err(RpcError::new(
                            ErrorCode::Conflict,
                            format!(
                                "cannot shut down while {} active turn(s) and {} queued message(s) are pending",
                                activity.active_turns, activity.ready_queued
                            ),
                        ));
                    }
                }
                tracing::info!("shutdown requested by a client");
                self.stop = true;
                reply::<m::CoreShutdown>(Empty {})
            }
            m::CoreConfigGet::NAME => {
                parse::<m::CoreConfigGet>(params)?;
                let config = k.core_config().await.map_err(kernel_err)?;
                reply::<m::CoreConfigGet>(config)
            }
            m::CoreConfigSet::NAME => {
                let p = parse::<m::CoreConfigSet>(params)?;
                let config = k.set_core_config(p.home_dir).await.map_err(kernel_err)?;
                reply::<m::CoreConfigSet>(config)
            }
            m::CoreWorktreesList::NAME => {
                parse::<m::CoreWorktreesList>(params)?;
                let list = k.list_managed_worktrees().await.map_err(kernel_err)?;
                reply::<m::CoreWorktreesList>(list)
            }
            m::CoreWorktreesDelete::NAME => {
                let p = parse::<m::CoreWorktreesDelete>(params)?;
                k.delete_managed_worktree(&p.path)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::CoreWorktreesDelete>(Empty {})
            }
            m::ProviderList::NAME => {
                parse::<m::ProviderList>(params)?;
                reply::<m::ProviderList>(ProviderListResult {
                    providers: k.provider_list(),
                })
            }
            m::UsageLimitsGet::NAME => {
                let p = parse::<m::UsageLimitsGet>(params)?;
                let providers = k
                    .usage_limits(p.provider.as_deref())
                    .await
                    .map_err(kernel_err)?;
                reply::<m::UsageLimitsGet>(m::UsageLimitsResult { providers })
            }
            m::UsageRefresh::NAME => {
                let p = parse::<m::UsageRefresh>(params)?;
                let limits = k.usage_refresh(&p.provider).await.map_err(kernel_err)?;
                reply::<m::UsageRefresh>(limits)
            }
            m::UsageSummaryGet::NAME => {
                let p = parse::<m::UsageSummaryGet>(params)?;
                let summary = k.usage_summary(p).await.map_err(kernel_err)?;
                reply::<m::UsageSummaryGet>(summary)
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
                            options: m.options,
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
            m::WorkspaceChat::NAME => {
                parse::<m::WorkspaceChat>(params)?;
                let workspace = k.workspace_chat().await.map_err(kernel_err)?;
                reply::<m::WorkspaceChat>(WorkspaceOpenResult { workspace })
            }
            m::WorkspaceClose::NAME => {
                let p = parse::<m::WorkspaceClose>(params)?;
                k.workspace_close(&p.workspace).await.map_err(kernel_err)?;
                reply::<m::WorkspaceClose>(Empty {})
            }
            m::WorkspaceDelete::NAME => {
                let p = parse::<m::WorkspaceDelete>(params)?;
                k.workspace_delete(&p.workspace).await.map_err(kernel_err)?;
                reply::<m::WorkspaceDelete>(Empty {})
            }
            m::WorkspaceGitStatus::NAME => {
                let p = parse::<m::WorkspaceGitStatus>(params)?;
                let info = k
                    .workspace_git_status(&p.workspace, p.path.as_deref())
                    .await
                    .map_err(kernel_err)?;
                reply::<m::WorkspaceGitStatus>(info)
            }
            m::WorkspaceGitCheckout::NAME => {
                let p = parse::<m::WorkspaceGitCheckout>(params)?;
                let info = k
                    .workspace_git_checkout(&p.workspace, &p.branch, p.path.as_deref())
                    .await
                    .map_err(kernel_err)?;
                reply::<m::WorkspaceGitCheckout>(info)
            }
            m::WorkspaceGitWorktreeCreate::NAME => {
                let p = parse::<m::WorkspaceGitWorktreeCreate>(params)?;
                let created = k
                    .workspace_git_worktree_create(
                        &p.workspace,
                        &p.branch,
                        p.base_branch.as_deref(),
                    )
                    .await
                    .map_err(kernel_err)?;
                reply::<m::WorkspaceGitWorktreeCreate>(created)
            }
            m::WorkspaceFilesList::NAME => {
                let p = parse::<m::WorkspaceFilesList>(params)?;
                let (files, truncated) = k
                    .workspace_files_list(
                        &p.workspace,
                        p.path.as_deref(),
                        p.query.as_deref(),
                        p.limit,
                    )
                    .await
                    .map_err(kernel_err)?;
                reply::<m::WorkspaceFilesList>(m::WorkspaceFilesListResult { files, truncated })
            }
            m::ThreadList::NAME => {
                let p = parse::<m::ThreadList>(params)?;
                let threads = k
                    .thread_list_opts(
                        &p.workspace,
                        p.include_archived.unwrap_or(false),
                        p.include_side.unwrap_or(false),
                    )
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadList>(ThreadListResult { threads })
            }
            m::ThreadCreate::NAME => {
                let p = parse::<m::ThreadCreate>(params)?;
                let thread = k
                    .thread_create_opts(
                        &p.workspace,
                        p.provider,
                        p.model,
                        p.title,
                        p.permission_mode,
                        p.options,
                        p.worktree_path,
                        p.parent,
                        p.side.unwrap_or(false),
                    )
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadCreate>(ThreadCreateResult { thread })
            }
            m::ThreadFork::NAME => {
                let p = parse::<m::ThreadFork>(params)?;
                let thread = k
                    .thread_fork(&p.thread, p.at_turn, p.before_turn, p.title)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadFork>(ThreadCreateResult { thread })
            }
            m::ThreadUpdate::NAME => {
                let p = parse::<m::ThreadUpdate>(params)?;
                let thread = k
                    .thread_update(
                        &p.thread,
                        p.provider,
                        p.model,
                        p.permission_mode,
                        p.title,
                        p.handoff,
                        p.options,
                    )
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadUpdate>(ThreadUpdateResult { thread })
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
            m::ThreadSteer::NAME => {
                let p = parse::<m::ThreadSteer>(params)?;
                let steered = k
                    .thread_steer(&p.thread, p.input)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadSteer>(steered)
            }
            m::ThreadDefer::NAME => {
                let p = parse::<m::ThreadDefer>(params)?;
                let entry = k
                    .thread_defer(&p.thread, p.input, p.trigger)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadDefer>(m::QueueEntryResult { entry })
            }
            m::ThreadQueueUpdate::NAME => {
                let p = parse::<m::ThreadQueueUpdate>(params)?;
                let entry = k
                    .queue_update(&p.thread, &p.entry, p.input)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadQueueUpdate>(m::QueueEntryResult { entry })
            }
            m::ThreadQueueRemove::NAME => {
                let p = parse::<m::ThreadQueueRemove>(params)?;
                k.queue_remove(&p.thread, &p.entry)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadQueueRemove>(Empty {})
            }
            m::ThreadQueueMove::NAME => {
                let p = parse::<m::ThreadQueueMove>(params)?;
                k.queue_move(&p.thread, &p.entry, p.before.as_ref())
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadQueueMove>(Empty {})
            }
            m::ThreadQueueSendNow::NAME => {
                let p = parse::<m::ThreadQueueSendNow>(params)?;
                let sent = k
                    .queue_send_now(&p.thread, &p.entry)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadQueueSendNow>(sent)
            }
            m::ThreadQueueResume::NAME => {
                let p = parse::<m::ThreadQueueResume>(params)?;
                k.queue_resume(&p.thread).await.map_err(kernel_err)?;
                reply::<m::ThreadQueueResume>(Empty {})
            }
            m::ThreadRespondApproval::NAME => {
                let p = parse::<m::ThreadRespondApproval>(params)?;
                // Local connections have no device id.
                k.respond_approval(&p.approval, p.decision, self.device.clone())
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ThreadRespondApproval>(Empty {})
            }
            m::ThreadArchive::NAME => {
                let p = parse::<m::ThreadArchive>(params)?;
                k.thread_archive(&p.thread).await.map_err(kernel_err)?;
                reply::<m::ThreadArchive>(Empty {})
            }
            m::ThreadUnarchive::NAME => {
                let p = parse::<m::ThreadUnarchive>(params)?;
                let thread = k.thread_unarchive(&p.thread).await.map_err(kernel_err)?;
                reply::<m::ThreadUnarchive>(ThreadUpdateResult { thread })
            }
            m::ThreadDelete::NAME => {
                let p = parse::<m::ThreadDelete>(params)?;
                k.thread_delete(&p.thread).await.map_err(kernel_err)?;
                reply::<m::ThreadDelete>(Empty {})
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
                    Topic::Usage => {
                        if p.after != 0 {
                            return Err(RpcError::new(
                                ErrorCode::InvalidParams,
                                "the usage topic has no log; subscribe with after: 0",
                            ));
                        }
                        (Deferred::Usage(k.usage_changes()), 0)
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
                let plugin = match (&p.path, &p.package, &p.registry) {
                    (Some(path), None, None) => crate::plugins::install(&self.host, path).await?,
                    (None, Some(package), None) => {
                        crate::plugins::install_package(&self.host, package).await?
                    }
                    (None, None, Some(r)) => self.registry()?.install(&self.host, r).await?,
                    _ => {
                        return Err(RpcError::new(
                            ErrorCode::InvalidParams,
                            "give exactly one of path, package and registry",
                        ));
                    }
                };
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
            m::PluginUpdates::NAME => {
                let p = parse::<m::PluginUpdates>(params)?;
                let updates = self
                    .registry()?
                    .updates(&self.host, p.refresh.unwrap_or(false))
                    .await?;
                reply::<m::PluginUpdates>(m::PluginUpdatesResult { updates })
            }
            m::PluginUpdate::NAME => {
                let p = parse::<m::PluginUpdate>(params)?;
                let plugin = self
                    .registry()?
                    .update(&self.host, &p.plugin, &p.version, p.grant.as_ref())
                    .await?;
                reply::<m::PluginUpdate>(PluginResult { plugin })
            }
            m::PluginLock::NAME => {
                parse::<m::PluginLock>(params)?;
                reply::<m::PluginLock>(m::PluginLockResult {
                    lock: self.host.manager().lock_text(),
                })
            }
            m::SecretSet::NAME => {
                let p = parse::<m::SecretSet>(params)?;
                check_secret_name(&p.name)?;
                if p.value.is_empty() {
                    return Err(RpcError::new(
                        ErrorCode::InvalidParams,
                        "a secret cannot be empty",
                    ));
                }
                k.secrets()
                    .set(&p.name, &p.value)
                    .await
                    .map_err(kernel_err)?;
                tracing::info!(device = ?self.device, name = %p.name, "secret set");
                reply::<m::SecretSet>(Empty {})
            }
            m::SecretDelete::NAME => {
                let p = parse::<m::SecretDelete>(params)?;
                check_secret_name(&p.name)?;
                k.secrets().delete(&p.name).await.map_err(kernel_err)?;
                tracing::info!(device = ?self.device, name = %p.name, "secret deleted");
                reply::<m::SecretDelete>(Empty {})
            }
            m::SecretList::NAME => {
                parse::<m::SecretList>(params)?;
                let secrets = self
                    .host
                    .manager()
                    .secret_list()
                    .await
                    .map_err(crate::plugins::host_err)?
                    .into_iter()
                    .map(|(name, set, plugins)| m::SecretInfo { name, set, plugins })
                    .collect();
                reply::<m::SecretList>(m::SecretListResult { secrets })
            }
            m::SettingsSchema::NAME => {
                let p = parse::<m::SettingsSchema>(params)?;
                let schema = self
                    .host
                    .manager()
                    .settings_schema(&p.plugin)
                    .await
                    .map_err(crate::plugins::host_err)?;
                reply::<m::SettingsSchema>(m::SettingsSchemaResult { schema })
            }
            m::SettingsGet::NAME => {
                let p = parse::<m::SettingsGet>(params)?;
                let values = self
                    .host
                    .manager()
                    .settings_get(&p.plugin)
                    .await
                    .map_err(crate::plugins::host_err)?;
                reply::<m::SettingsGet>(m::SettingsGetResult { values })
            }
            m::SettingsSet::NAME => {
                let p = parse::<m::SettingsSet>(params)?;
                self.host
                    .manager()
                    .settings_set(&p.plugin, &p.values)
                    .await
                    .map_err(crate::plugins::host_err)?;
                reply::<m::SettingsSet>(Empty {})
            }
            m::RegistryList::NAME => {
                parse::<m::RegistryList>(params)?;
                let registries = self.registry()?.list().await?;
                reply::<m::RegistryList>(m::RegistryListResult { registries })
            }
            m::RegistryAdd::NAME => {
                let p = parse::<m::RegistryAdd>(params)?;
                self.registry()?.add(&p.url).await?;
                reply::<m::RegistryAdd>(Empty {})
            }
            m::RegistryRemove::NAME => {
                let p = parse::<m::RegistryRemove>(params)?;
                self.registry()?.remove(&p.url).await?;
                reply::<m::RegistryRemove>(Empty {})
            }
            m::RegistrySearch::NAME => {
                let p = parse::<m::RegistrySearch>(params)?;
                let result = self
                    .registry()?
                    .search(p.query.as_deref(), p.refresh.unwrap_or(false))
                    .await?;
                reply::<m::RegistrySearch>(result)
            }
            m::AuthPair::NAME | m::AuthToken::NAME => Err(match self.device {
                None => RpcError::new(
                    ErrorCode::Unsupported,
                    "local connections need no authentication",
                ),
                Some(_) => RpcError::new(ErrorCode::Conflict, "already authenticated"),
            }),
            m::AuthPairingCreate::NAME => {
                parse::<m::AuthPairingCreate>(params)?;
                if self.device.is_some() {
                    return Err(RpcError::new(
                        ErrorCode::PermissionDenied,
                        "pairing codes are made on the core's machine (kumikit pair)",
                    ));
                }
                let Some(remote) = &self.info.remote else {
                    return Err(RpcError::new(
                        ErrorCode::Unsupported,
                        "remote access is off; start the core with --listen",
                    ));
                };
                let code = remote
                    .create_code()
                    .map_err(|e| RpcError::new(ErrorCode::Internal, e.to_string()))?;
                reply::<m::AuthPairingCreate>(code)
            }
            m::AuthDevicesList::NAME => {
                parse::<m::AuthDevicesList>(params)?;
                let devices = k
                    .device_list()
                    .await
                    .map_err(kernel_err)?
                    .into_iter()
                    .map(|d| device_info(d, self.device.as_deref()))
                    .collect();
                reply::<m::AuthDevicesList>(m::DeviceListResult { devices })
            }
            m::AuthDevicesRevoke::NAME => {
                let p = parse::<m::AuthDevicesRevoke>(params)?;
                k.device_revoke(&p.device).await.map_err(kernel_err)?;
                reply::<m::AuthDevicesRevoke>(Empty {})
            }
            m::PluginWorkspaces::NAME => {
                let p = parse::<m::PluginWorkspaces>(params)?;
                self.host
                    .manager()
                    .get(&p.plugin)
                    .map_err(crate::plugins::host_err)?;
                let workspaces = k.plugin_workspaces(&p.plugin).await.map_err(kernel_err)?;
                reply::<m::PluginWorkspaces>(m::PluginWorkspacesResult { workspaces })
            }
            m::PluginSetWorkspaces::NAME => {
                let p = parse::<m::PluginSetWorkspaces>(params)?;
                self.host
                    .manager()
                    .get(&p.plugin)
                    .map_err(crate::plugins::host_err)?;
                k.plugin_set_workspaces(&p.plugin, &p.workspaces)
                    .await
                    .map_err(kernel_err)?;
                tracing::info!(device = ?self.device, plugin = %p.plugin, count = p.workspaces.len(), "linked workspaces set");
                reply::<m::PluginSetWorkspaces>(Empty {})
            }
            m::ScheduleList::NAME => {
                parse::<m::ScheduleList>(params)?;
                let schedules = k
                    .schedule_list()
                    .await
                    .map_err(kernel_err)?
                    .into_iter()
                    .map(|r| m::ScheduleInfo {
                        missed: match r.spec.missed {
                            kumikit_kernel::schedule::Missed::Skip => "skip".into(),
                            kumikit_kernel::schedule::Missed::Once => "once".into(),
                        },
                        source: match r.source {
                            kumikit_kernel::schedule::Source::Manifest => "manifest".into(),
                            kumikit_kernel::schedule::Source::Plugin => "plugin".into(),
                        },
                        plugin: r.plugin,
                        key: r.key,
                        description: r.spec.description,
                        cron: r.spec.cron,
                        time_zone: r.spec.zone,
                        enabled: r.enabled,
                        next_run: r.next_run,
                        last_run: r.last_run,
                        last_outcome: r.last_outcome,
                    })
                    .collect();
                reply::<m::ScheduleList>(m::ScheduleListResult { schedules })
            }
            m::ScheduleSetEnabled::NAME => {
                let p = parse::<m::ScheduleSetEnabled>(params)?;
                k.schedule_set_enabled(&p.plugin, &p.key, p.enabled, jiff::Timestamp::now())
                    .await
                    .map_err(kernel_err)?;
                tracing::info!(device = ?self.device, plugin = %p.plugin, key = %p.key, enabled = p.enabled, "schedule changed");
                reply::<m::ScheduleSetEnabled>(Empty {})
            }
            m::ScheduleDelete::NAME => {
                let p = parse::<m::ScheduleDelete>(params)?;
                if p.key.starts_with(kumikit_kernel::schedule::MANIFEST_PREFIX) {
                    return Err(RpcError::new(
                        ErrorCode::Conflict,
                        "a manifest schedule cannot be deleted; pause it instead",
                    ));
                }
                k.schedule_delete(&p.plugin, &p.key)
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ScheduleDelete>(Empty {})
            }
            m::ScheduleRun::NAME => {
                let p = parse::<m::ScheduleRun>(params)?;
                k.schedule_run_now(&p.plugin, &p.key, jiff::Timestamp::now())
                    .await
                    .map_err(kernel_err)?;
                reply::<m::ScheduleRun>(Empty {})
            }
            m::GatewayAttach::NAME => Err(RpcError::new(
                ErrorCode::InvalidRequest,
                "gateway.attach must be a local connection's first message",
            )),
            m::CommandList::NAME => {
                parse::<m::CommandList>(params)?;
                let commands = self
                    .host
                    .commands()
                    .into_iter()
                    .map(|c| m::CommandInfo {
                        plugin: c.plugin,
                        id: c.id,
                        title: c.title,
                        scope: if c.workspace {
                            m::Scope::Workspace
                        } else {
                            m::Scope::Core
                        },
                        when: c.when,
                    })
                    .collect();
                reply::<m::CommandList>(m::CommandListResult { commands })
            }
            m::CommandRun::NAME => {
                let p = parse::<m::CommandRun>(params)?;
                let scoped = self
                    .host
                    .command_scope(&p.plugin, &p.command)
                    .map_err(crate::plugins::host_err)?;
                let workspace = self.acting_workspace(scoped, p.workspace.as_ref()).await?;
                let args = p.args.unwrap_or_else(|| Value::Object(Default::default()));
                let (message, view) = self
                    .host
                    .run_command(&p.plugin, &p.command, &args, workspace)
                    .await
                    .map_err(crate::plugins::host_err)?;
                reply::<m::CommandRun>(m::CommandResult { message, view })
            }
            other => match other.split_once('/') {
                Some((plugin, name)) => self.plugin_method(plugin, name, params).await,
                None => Err(RpcError::new(
                    ErrorCode::MethodNotFound,
                    format!("no method {other}"),
                )),
            },
        }
    }

    /// The workspace a `workspace`-scoped call acts for: required, open, with its root.
    async fn acting_workspace(
        &self,
        scoped: bool,
        workspace: Option<&WorkspaceId>,
    ) -> Result<Option<kumikit_plugin_host::runtime::WorkspaceRef>, RpcError> {
        if !scoped {
            return Ok(None);
        }
        let id = workspace.ok_or_else(|| {
            RpcError::new(ErrorCode::InvalidParams, "this call needs a workspace")
        })?;
        let ws = self.kernel.workspace_get(id).await.map_err(kernel_err)?;
        Ok(Some(kumikit_plugin_host::runtime::WorkspaceRef {
            id: ws.id.0,
            root: ws.root,
        }))
    }

    /// `<plugin id>/<name>`: a method the enabled plugin declares
    /// (`docs/spec/02-protocol.md#methods-added-in-06`).
    async fn plugin_method(
        &self,
        plugin: &str,
        name: &str,
        params: Option<Value>,
    ) -> Result<Value, RpcError> {
        let Some(scoped) = self.host.method_scope(plugin, name) else {
            return Err(RpcError::new(
                ErrorCode::MethodNotFound,
                format!("no method {plugin}/{name}"),
            ));
        };
        let params = params.unwrap_or_else(|| Value::Object(Default::default()));
        let workspace = match params.get("workspace") {
            Some(Value::String(w)) => Some(WorkspaceId(w.clone())),
            _ => None,
        };
        let workspace = self.acting_workspace(scoped, workspace.as_ref()).await?;
        self.host
            .call_method(plugin, name, &params, workspace)
            .await
            .map_err(crate::plugins::host_err)
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
            Deferred::Usage(rx) => tokio::spawn(pump_usage(
                self.kernel.clone(),
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

/// Forwards live changes to the core's usage data. Nothing is logged: `seq` counts within
/// this subscription. A listener that lags re-sends every provider's limits as
/// `usage.limits`, so the client converges; it refetches statistics on any event.
async fn pump_usage(
    kernel: Kernel,
    mut rx: broadcast::Receiver<UsageEvent>,
    topic: String,
    tx: mpsc::Sender<String>,
) {
    let mut seq = 0u64;
    loop {
        let events = match rx.recv().await {
            Ok(event) => vec![event],
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                tracing::debug!(missed, "usage listener lagged; resending limits");
                match kernel.usage_limits(None).await {
                    Ok(all) => all.into_iter().map(UsageEvent::Limits).collect(),
                    Err(e) => {
                        tracing::warn!(error = %e, "cannot resend limits");
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
                    tracing::error!(error = %err, "failed to encode an event");
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

#[cfg(test)]
mod tests;
