//! The kernel's half of the tool gateway (`docs/spec/03-kernel.md#tool-gateway`, ADR
//! 0025): per-session tokens, the `mcp-server` record a provider session gets, and the
//! log side of a gateway call (its `tool.*` events and its approval). The MCP server itself
//! is in the core; the tools are run by the plugin host.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use kumikit_protocol::ThreadEvent;
use kumikit_protocol::event::{
    ApprovalRequested, ToolFinished, ToolOutput, ToolSource, ToolStarted,
};
use kumikit_protocol::model::{
    Approval, ApprovalId, ApprovalStatus, Resolution, ThreadId, TurnId, WorkspaceId,
};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::error::{KernelError, KernelResult};
use crate::kernel::{Kernel, lock};
use crate::provider::McpServer;

/// The environment variable that carries a session's token to `kumikit mcp`.
pub const TOKEN_VAR: &str = "KUMIKIT_MCP_TOKEN";

/// How agents reach the gateway: the program and arguments of `kumikit mcp`. Set by
/// `kumikit serve`; without it no session gets a gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayConfig {
    /// Absolute path of the core binary.
    pub command: String,
    /// Its arguments (`mcp --socket <socket>`).
    pub args: Vec<String>,
}

/// What a token is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewaySession {
    /// The thread whose provider session got the token.
    pub thread: ThreadId,
    /// Its workspace.
    pub workspace: WorkspaceId,
    /// The workspace root, for the tools to act in.
    pub workspace_root: String,
}

#[derive(Default)]
pub(crate) struct State {
    config: OnceLock<GatewayConfig>,
    tokens: Mutex<HashMap<String, GatewaySession>>,
    by_thread: Mutex<HashMap<ThreadId, String>>,
    waiters: Mutex<HashMap<ApprovalId, oneshot::Sender<Resolution>>>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("gateway::State")
            .field("configured", &self.config.get().is_some())
            .field("sessions", &lock(&self.tokens).len())
            .finish_non_exhaustive()
    }
}

fn new_token() -> KernelResult<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| KernelError::Io(format!("no randomness: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

impl Kernel {
    fn gateway(&self) -> &State {
        &self.inner.gateway
    }

    /// Turns the gateway on (`kumikit serve`).
    pub fn set_gateway(&self, config: GatewayConfig) {
        let _ = self.gateway().config.set(config);
    }

    /// Whether the plugin's grant includes `tools`.
    async fn may_hand_out_tools(&self, plugin: &str) -> bool {
        matches!(
            self.plugin_get(plugin).await,
            Ok(Some(r)) if r.granted.as_ref().is_some_and(|g| g.tools)
        )
    }

    /// A gateway for a new provider session of `thread`, if the gateway is on and the
    /// provider's (contributing) plugin is granted `tools`. Replaces the thread's earlier
    /// token.
    pub(crate) async fn gateway_for_session(
        &self,
        thread: &ThreadId,
        workspace: &WorkspaceId,
        workspace_root: &str,
        contributor: &str,
    ) -> KernelResult<Option<McpServer>> {
        let Some(config) = self.gateway().config.get().cloned() else {
            return Ok(None);
        };
        if !self.may_hand_out_tools(contributor).await {
            self.gateway_revoke(thread);
            return Ok(None);
        }
        let token = new_token()?;
        self.gateway_revoke(thread);
        lock(&self.gateway().tokens).insert(
            token.clone(),
            GatewaySession {
                thread: thread.clone(),
                workspace: workspace.clone(),
                workspace_root: workspace_root.to_owned(),
            },
        );
        lock(&self.gateway().by_thread).insert(thread.clone(), token.clone());
        Ok(Some(McpServer {
            name: "kumikit".into(),
            command: config.command,
            args: config.args,
            env: vec![(TOKEN_VAR.to_owned(), token)],
        }))
    }

    /// Voids the thread's token (its session ended).
    pub(crate) fn gateway_revoke(&self, thread: &ThreadId) {
        if let Some(token) = lock(&self.gateway().by_thread).remove(thread) {
            lock(&self.gateway().tokens).remove(&token);
        }
    }

    /// The session a token is bound to, if it is still valid.
    pub fn gateway_session(&self, token: &str) -> Option<GatewaySession> {
        lock(&self.gateway().tokens).get(token).cloned()
    }

    /// The turn running on a thread now, if any.
    pub fn running_turn(&self, thread: &ThreadId) -> Option<TurnId> {
        self.lock_runs().get(thread).map(|r| r.turn.clone())
    }

    /// Appends `tool.started` for a gateway call in the running `turn`.
    pub async fn gateway_tool_started(
        &self,
        thread: &ThreadId,
        turn: &TurnId,
        call: &str,
        name: &str,
        input: Value,
        source: ToolSource,
    ) -> KernelResult<()> {
        self.append(
            thread,
            ThreadEvent::ToolStarted(ToolStarted {
                turn: turn.clone(),
                call: call.to_owned(),
                name: name.to_owned(),
                input,
                source: Some(source),
            }),
        )
        .await
        .map(|_| ())
    }

    /// Appends `tool.output` and `tool.finished` for a gateway call.
    pub async fn gateway_tool_finished(
        &self,
        thread: &ThreadId,
        turn: &TurnId,
        call: &str,
        output: Option<String>,
        ok: bool,
        result: Option<Value>,
    ) -> KernelResult<()> {
        if let Some(chunk) = output.filter(|o| !o.is_empty()) {
            self.append(
                thread,
                ThreadEvent::ToolOutput(ToolOutput {
                    turn: turn.clone(),
                    call: call.to_owned(),
                    chunk,
                }),
            )
            .await?;
        }
        self.append(
            thread,
            ThreadEvent::ToolFinished(ToolFinished {
                turn: turn.clone(),
                call: call.to_owned(),
                ok,
                result,
            }),
        )
        .await
        .map(|_| ())
    }

    /// Asks for an approval in the running turn and waits for the answer, or
    /// `cancelled` when the turn ends first.
    pub async fn gateway_request_approval(
        &self,
        thread: &ThreadId,
        turn: &TurnId,
        summary: String,
        detail: Value,
    ) -> KernelResult<Resolution> {
        let id = ApprovalId(self.new_id());
        let (tx, rx) = oneshot::channel();
        lock(&self.gateway().waiters).insert(id.clone(), tx);
        let approval = Approval {
            id: id.clone(),
            thread: thread.clone(),
            turn: turn.clone(),
            kind: "tool".into(),
            summary,
            detail: Some(detail),
            status: ApprovalStatus::Pending,
        };
        if let Err(e) = self
            .append(
                thread,
                ThreadEvent::ApprovalRequested(ApprovalRequested { approval }),
            )
            .await
        {
            lock(&self.gateway().waiters).remove(&id);
            return Err(e);
        }
        Ok(rx.await.unwrap_or(Resolution::Cancelled))
    }

    /// Hands a resolution to a gateway call waiting on it; `false` if none waits (the
    /// approval is a provider's).
    pub(crate) fn gateway_resolved(&self, approval: &ApprovalId, resolution: Resolution) -> bool {
        match lock(&self.gateway().waiters).remove(approval) {
            Some(tx) => {
                let _ = tx.send(resolution);
                true
            }
            None => false,
        }
    }

    /// Whether the user answered `allow_always` to this tool earlier in the thread
    /// (folded from the log, so it survives restarts).
    pub async fn gateway_always_allowed(
        &self,
        thread: &ThreadId,
        tool: &str,
    ) -> KernelResult<bool> {
        let mut sub = self.subscribe(thread, 0).await?;
        let head = sub.head();
        let mut asked: Vec<ApprovalId> = Vec::new();
        let mut seen = 0;
        while seen < head {
            let Some(batch) = sub.next_batch().await? else {
                break;
            };
            for e in batch {
                seen = e.seq;
                match e.event {
                    ThreadEvent::ApprovalRequested(r)
                        if r.approval.kind == "tool"
                            && r.approval
                                .detail
                                .as_ref()
                                .and_then(|d| d.get("tool"))
                                .and_then(Value::as_str)
                                == Some(tool) =>
                    {
                        asked.push(r.approval.id);
                    }
                    ThreadEvent::ApprovalResolved(r)
                        if r.decision == Resolution::AllowAlways && asked.contains(&r.approval) =>
                    {
                        return Ok(true);
                    }
                    _ => {}
                }
            }
        }
        Ok(false)
    }
}
