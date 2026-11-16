//! The internal provider interface. It mirrors the `kumikit:provider` WIT world
//! (`docs/spec/05-providers.md`) so the M1 WASM adapter can sit behind the same trait.
//!
//! Providers never append to the log directly. They emit through an [`EventSink`], whose
//! [`ProviderEvent`] type cannot express kernel-only kinds (`turn.started`, `turn.ended`,
//! `approval.resolved`), and which stamps the thread and turn itself.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kumikit_protocol::ThreadEvent;
use kumikit_protocol::event::{
    ApprovalRequested, FileChanged, PluginEvent, ToolFinished, ToolOutput, ToolStarted, TurnText,
    Usage,
};
use kumikit_protocol::model::{
    Approval, ApprovalId, ApprovalStatus, Decision, ThreadId, TurnId, UserInput,
};
use serde_json::Value;

use crate::error::{KernelError, KernelResult};
use crate::kernel::Kernel;

/// A boxed, sendable future, so [`Provider`] can be used as a trait object.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A provider failure, reported as `plugin_failed` or as a failed turn.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ProviderError {
    /// What went wrong, for the user.
    pub message: String,
}

impl ProviderError {
    /// An error with a message.
    pub fn new(message: impl Into<String>) -> Self {
        ProviderError {
            message: message.into(),
        }
    }
}

/// Result alias for provider calls.
pub type ProviderResult<T> = Result<T, ProviderError>;

/// A model a provider offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    /// Model id, passed back in [`SessionConfig::model`].
    pub id: String,
    /// Display label.
    pub label: String,
    /// Capability strings (`docs/spec/05-providers.md#capabilities`).
    pub capabilities: Vec<String>,
}

/// What a provider needs to start a session for a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionConfig {
    /// The thread.
    pub thread: ThreadId,
    /// Absolute root of the thread's workspace.
    pub workspace_root: String,
    /// Model id.
    pub model: String,
    /// The provider's own session id from an earlier run, if any.
    pub resume: Option<String>,
    /// Tool gateway URL; always `None` until the gateway exists.
    pub mcp_endpoint: Option<String>,
    /// Provider-specific options.
    pub options: Vec<(String, String)>,
}

/// An agent provider. Methods take owned arguments so the futures borrow only `self`.
pub trait Provider: Send + Sync + fmt::Debug {
    /// Provider id, unique on this core.
    fn id(&self) -> &str;
    /// The plugin named in this provider's `plugin.event`s. Defaults to [`Provider::id`];
    /// plugin-backed providers return the id of the plugin whose code emits them.
    fn plugin_id(&self) -> &str {
        self.id()
    }
    /// Display name. Defaults to [`Provider::id`].
    fn name(&self) -> &str {
        self.id()
    }
    /// The plugin whose manifest declares this provider. Defaults to
    /// [`Provider::plugin_id`]; delegated providers return the contributing plugin.
    fn contributor(&self) -> &str {
        self.plugin_id()
    }
    /// Models this provider offers; the first is the default.
    fn list_models(&self) -> BoxFuture<'_, ProviderResult<Vec<Model>>>;
    /// Starts (or resumes) a session; returns the provider's session id.
    fn start_session(&self, config: SessionConfig) -> BoxFuture<'_, ProviderResult<String>>;
    /// Runs one turn. Returns when the turn is over; events go to `sink` meanwhile.
    fn send(
        &self,
        session: String,
        input: UserInput,
        sink: EventSink,
    ) -> BoxFuture<'_, ProviderResult<()>>;
    /// Delivers the answer to an approval the provider requested through its sink.
    ///
    /// Clients see `approval.requested` as soon as it commits, so this can arrive before
    /// [`EventSink::request_approval`] has returned to the provider; providers must keep
    /// an early answer until they wait for it.
    fn respond_approval(
        &self,
        session: String,
        approval: ApprovalId,
        decision: Decision,
    ) -> BoxFuture<'_, ProviderResult<()>>;
    /// Asks the provider to stop the running turn. The kernel also drops the `send`
    /// future afterwards, so a provider that ignores this cannot keep a turn open.
    fn cancel(&self, session: String) -> BoxFuture<'_, ()>;
    /// Ends a session.
    fn end_session(&self, session: String) -> BoxFuture<'_, ()>;
}

/// An event a provider may emit. Kernel-only kinds are not representable.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderEvent {
    /// `text.delta`
    TextDelta {
        /// Text to append.
        text: String,
    },
    /// `text.done`
    TextDone {
        /// The full message.
        text: String,
    },
    /// `reasoning.delta`
    ReasoningDelta {
        /// Text to append.
        text: String,
    },
    /// `tool.started`
    ToolStarted {
        /// Call id, unique within the turn.
        call: String,
        /// Tool name.
        name: String,
        /// Tool input.
        input: Value,
    },
    /// `tool.output`
    ToolOutput {
        /// Call id.
        call: String,
        /// Output text.
        chunk: String,
    },
    /// `tool.finished`
    ToolFinished {
        /// Call id.
        call: String,
        /// Whether it succeeded.
        ok: bool,
        /// Result, if any.
        result: Option<Value>,
    },
    /// `file.changed`
    FileChanged {
        /// Path relative to the workspace root.
        path: String,
        /// Unified diff, if known.
        diff: Option<String>,
    },
    /// `usage`
    Usage {
        /// Input tokens.
        input_tokens: u64,
        /// Output tokens.
        output_tokens: u64,
        /// Cost in US dollars, if known.
        cost: Option<f64>,
    },
    /// `plugin.event`; the kernel fills in the plugin id.
    Plugin {
        /// The provider's own kind.
        kind: String,
        /// Its data.
        data: Value,
    },
}

/// Where a provider sends the events of one turn. Closed by the kernel when the turn
/// ends; later emits fail with `conflict`.
#[derive(Clone)]
pub struct EventSink {
    kernel: Kernel,
    thread: ThreadId,
    turn: TurnId,
    /// Stamped on `plugin.event`.
    plugin: String,
    open: Arc<AtomicBool>,
}

impl fmt::Debug for EventSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventSink")
            .field("thread", &self.thread)
            .field("turn", &self.turn)
            .field("open", &self.open.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl EventSink {
    pub(crate) fn new(kernel: Kernel, thread: ThreadId, turn: TurnId, plugin: String) -> Self {
        EventSink {
            kernel,
            thread,
            turn,
            plugin,
            open: Arc::new(AtomicBool::new(true)),
        }
    }

    /// The turn this sink belongs to.
    pub fn turn(&self) -> &TurnId {
        &self.turn
    }

    pub(crate) fn close(&self) {
        self.open.store(false, Ordering::Release);
    }

    fn check_open(&self) -> KernelResult<()> {
        if self.open.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(KernelError::Conflict(format!(
                "turn {} has ended",
                self.turn
            )))
        }
    }

    /// Appends an event to the turn; returns its `seq`.
    pub async fn emit(&self, event: ProviderEvent) -> KernelResult<u64> {
        self.check_open()?;
        let turn = self.turn.clone();
        let event = match event {
            ProviderEvent::TextDelta { text } => ThreadEvent::TextDelta(TurnText { turn, text }),
            ProviderEvent::TextDone { text } => ThreadEvent::TextDone(TurnText { turn, text }),
            ProviderEvent::ReasoningDelta { text } => {
                ThreadEvent::ReasoningDelta(TurnText { turn, text })
            }
            ProviderEvent::ToolStarted { call, name, input } => {
                ThreadEvent::ToolStarted(ToolStarted {
                    turn,
                    call,
                    name,
                    input,
                })
            }
            ProviderEvent::ToolOutput { call, chunk } => {
                ThreadEvent::ToolOutput(ToolOutput { turn, call, chunk })
            }
            ProviderEvent::ToolFinished { call, ok, result } => {
                ThreadEvent::ToolFinished(ToolFinished {
                    turn,
                    call,
                    ok,
                    result,
                })
            }
            ProviderEvent::FileChanged { path, diff } => {
                ThreadEvent::FileChanged(FileChanged { turn, path, diff })
            }
            ProviderEvent::Usage {
                input_tokens,
                output_tokens,
                cost,
            } => ThreadEvent::Usage(Usage {
                turn,
                input_tokens,
                output_tokens,
                cost,
            }),
            ProviderEvent::Plugin { kind, data } => ThreadEvent::PluginEvent(PluginEvent {
                plugin: self.plugin.clone(),
                kind,
                data,
            }),
        };
        self.kernel.append(&self.thread, event).await
    }

    /// Asks for an approval. The kernel assigns its id and appends
    /// `approval.requested`; the answer arrives through [`Provider::respond_approval`].
    pub async fn request_approval(
        &self,
        kind: impl Into<String>,
        summary: impl Into<String>,
        detail: Option<Value>,
    ) -> KernelResult<ApprovalId> {
        self.check_open()?;
        let id = ApprovalId(self.kernel.new_id());
        let approval = Approval {
            id: id.clone(),
            thread: self.thread.clone(),
            turn: self.turn.clone(),
            kind: kind.into(),
            summary: summary.into(),
            detail,
            status: ApprovalStatus::Pending,
        };
        self.kernel
            .append(
                &self.thread,
                ThreadEvent::ApprovalRequested(ApprovalRequested { approval }),
            )
            .await?;
        Ok(id)
    }
}
