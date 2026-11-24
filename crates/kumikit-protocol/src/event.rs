//! Thread events (`docs/spec/03-kernel.md#thread-events`), workspace events
//! (`docs/spec/02-protocol.md#workspace-topic`) and the `events.event` notification that
//! carries them.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::model::{
    Approval, ApprovalId, HandoffMode, ProviderModel, QueueEntryId, Resolution, Thread, Timestamp,
    TurnId, TurnOutcome, TurnPurpose, UserInput,
};

/// `turn.started`: the kernel started a turn for a user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TurnStarted {
    /// The new turn.
    pub turn: TurnId,
    /// The user message that started it.
    pub input: UserInput,
    /// Who sent it, when a plugin did (through `kumikit:host/client`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<MessageFrom>,
    /// The queue entry it came from; starting the turn removes the entry (since 0.7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub entry: Option<QueueEntryId>,
    /// Qualified provider id running this turn (since 0.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub provider: Option<String>,
    /// Model id running this turn (since 0.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<String>,
    /// Special purpose, when started by the kernel (since 0.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub purpose: Option<TurnPurpose>,
}

/// Who sent a message that a plugin sent for a user (a bot, a schedule).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessageFrom {
    /// The plugin.
    pub plugin: String,
}

/// `text.delta`, `text.done` and `reasoning.delta`: a piece of text in a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TurnText {
    /// The turn.
    pub turn: TurnId,
    /// A delta, or for `text.done` the full message.
    pub text: String,
}

/// `tool.started`: the agent called a tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ToolStarted {
    /// The turn.
    pub turn: TurnId,
    /// Call id, unique within the turn; chosen by the provider.
    pub call: String,
    /// Tool name.
    pub name: String,
    /// Tool input.
    pub input: Value,
    /// For a call through the tool gateway: the plugin and tool that served it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub source: Option<ToolSource>,
}

/// Which plugin's tool served a gateway call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ToolSource {
    /// The plugin.
    pub plugin: String,
    /// The tool's name in its manifest.
    pub tool: String,
}

/// `tool.output`: a chunk of a tool call's output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutput {
    /// The turn.
    pub turn: TurnId,
    /// Call id.
    pub call: String,
    /// Output text.
    pub chunk: String,
}

/// `tool.finished`: a tool call ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ToolFinished {
    /// The turn.
    pub turn: TurnId,
    /// Call id.
    pub call: String,
    /// Whether the call succeeded.
    pub ok: bool,
    /// The result, if the tool returned one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

/// `file.changed`: the agent changed a file in the workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct FileChanged {
    /// The turn.
    pub turn: TurnId,
    /// Path relative to the workspace root.
    pub path: String,
    /// Unified diff, if the provider has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

/// `approval.requested`: the agent waits for an answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequested {
    /// The approval, with status `pending`.
    pub approval: Approval,
}

/// `approval.resolved`: an approval was answered or cancelled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApprovalResolved {
    /// The approval.
    pub approval: ApprovalId,
    /// The answer, or `cancelled`.
    pub decision: Resolution,
    /// Device id of the client that answered; absent for local connections and the kernel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// The plugin that answered, through `kumikit:host/client`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub plugin: Option<String>,
}

/// `usage`: tokens (and optionally cost) used by a turn so far.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Usage {
    /// The turn.
    pub turn: TurnId,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Cost in US dollars, if the provider knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

/// `turn.ended`: the kernel ended a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct TurnEnded {
    /// The turn.
    pub turn: TurnId,
    /// How it ended.
    pub outcome: TurnOutcome,
    /// Why it failed, for `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `plugin.event`: an event kind defined by a plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginEvent {
    /// The plugin id.
    pub plugin: String,
    /// The plugin's own kind.
    pub kind: String,
    /// The plugin's data.
    pub data: Value,
}

/// A message waiting in a thread's queue (`docs/spec/03-kernel.md#the-queue`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    /// The entry.
    pub id: QueueEntryId,
    /// The turn it will run as.
    pub turn: TurnId,
    /// The message.
    pub input: UserInput,
    /// Who sent it, when a plugin did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<MessageFrom>,
}

/// Why a thread's queue waits instead of starting its next entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QueueHold {
    /// The user stopped the running turn.
    Stopped,
    /// The running turn failed.
    Failed,
    /// The core restarted with messages in the queue.
    Restarted,
}

impl QueueHold {
    /// The wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            QueueHold::Stopped => "stopped",
            QueueHold::Failed => "failed",
            QueueHold::Restarted => "restarted",
        }
    }

    /// Parses the wire string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "stopped" => Some(QueueHold::Stopped),
            "failed" => Some(QueueHold::Failed),
            "restarted" => Some(QueueHold::Restarted),
            _ => None,
        }
    }
}

/// Why an entry left the queue without starting a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QueueRemoval {
    /// A client or plugin removed it.
    Removed,
}

/// Why a message joined the queue other than by `thread.send` during a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QueueAddReason {
    /// It was sent with `thread.steer` but the running turn did not take it: the model
    /// cannot be steered, it had attachments, or the turn ended first.
    NotSteered,
}

/// `queue.added`: a message joined the queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueAdded {
    /// The new entry.
    pub entry: QueueEntry,
    /// True if it went to the front of the queue instead of the back.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub front: bool,
    /// Why, when it was not an ordinary queued message (since 0.8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<QueueAddReason>,
}

/// `input.steered`: the running turn's provider took a message sent with
/// `thread.steer` and passed it to its agent (ADR 0028).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct InputSteered {
    /// The turn that took it.
    pub turn: TurnId,
    /// The message.
    pub input: UserInput,
}

/// `thread.switched`: the thread switched to another provider (ADR 0029, since 0.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSwitched {
    /// The outgoing provider and model.
    pub from: ProviderModel,
    /// The incoming provider and model.
    pub to: ProviderModel,
    /// Requested handoff mode.
    pub handoff: HandoffMode,
    /// True when `summary` mode fell back to `transcript` because the summary turn failed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub summary_failed: bool,
}

/// `handoff.delivered`: the bounded earlier conversation was delivered to the new
/// provider on a completed turn (ADR 0029, since 0.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HandoffDelivered {
    /// The turn that carried the handoff context.
    pub turn: TurnId,
    /// The outgoing provider and model.
    pub from: ProviderModel,
    /// The incoming provider and model.
    pub to: ProviderModel,
    /// Number of earlier user/assistant messages included.
    pub messages: u32,
    /// True when older messages were omitted to stay within the 200-message / 64 KiB cap.
    pub truncated: bool,
    /// The exact context preamble carried forward, for disclosure in the UI.
    pub rendered: String,
}

/// `queue.updated`: a queued message was edited.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueUpdated {
    /// The entry after the edit.
    pub entry: QueueEntry,
}

/// `queue.removed`: an entry left the queue without starting a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueRemoved {
    /// The entry.
    pub entry: QueueEntryId,
    /// Why.
    pub reason: QueueRemoval,
}

/// `queue.moved`: the queue's order changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueMoved {
    /// Every entry, in the new order.
    pub order: Vec<QueueEntryId>,
}

/// `queue.held`: the queue waits for `thread.queue.resume`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueHeld {
    /// Why.
    pub reason: QueueHold,
}

/// `queue.resumed`: a held queue may start its entries again.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct QueueResumed {}

/// A thread event: `kind` plus `data`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data")]
pub enum ThreadEvent {
    /// `turn.started`
    #[serde(rename = "turn.started")]
    TurnStarted(TurnStarted),
    /// `text.delta`
    #[serde(rename = "text.delta")]
    TextDelta(TurnText),
    /// `text.done`
    #[serde(rename = "text.done")]
    TextDone(TurnText),
    /// `reasoning.delta`
    #[serde(rename = "reasoning.delta")]
    ReasoningDelta(TurnText),
    /// `tool.started`
    #[serde(rename = "tool.started")]
    ToolStarted(ToolStarted),
    /// `tool.output`
    #[serde(rename = "tool.output")]
    ToolOutput(ToolOutput),
    /// `tool.finished`
    #[serde(rename = "tool.finished")]
    ToolFinished(ToolFinished),
    /// `file.changed`
    #[serde(rename = "file.changed")]
    FileChanged(FileChanged),
    /// `approval.requested`
    #[serde(rename = "approval.requested")]
    ApprovalRequested(ApprovalRequested),
    /// `approval.resolved`
    #[serde(rename = "approval.resolved")]
    ApprovalResolved(ApprovalResolved),
    /// `usage`
    #[serde(rename = "usage")]
    Usage(Usage),
    /// `turn.ended`
    #[serde(rename = "turn.ended")]
    TurnEnded(TurnEnded),
    /// `plugin.event`
    #[serde(rename = "plugin.event")]
    PluginEvent(PluginEvent),
    /// `queue.added`
    #[serde(rename = "queue.added")]
    QueueAdded(QueueAdded),
    /// `queue.updated`
    #[serde(rename = "queue.updated")]
    QueueUpdated(QueueUpdated),
    /// `queue.removed`
    #[serde(rename = "queue.removed")]
    QueueRemoved(QueueRemoved),
    /// `queue.moved`
    #[serde(rename = "queue.moved")]
    QueueMoved(QueueMoved),
    /// `queue.held`
    #[serde(rename = "queue.held")]
    QueueHeld(QueueHeld),
    /// `queue.resumed`
    #[serde(rename = "queue.resumed")]
    QueueResumed(QueueResumed),
    /// `input.steered`
    #[serde(rename = "input.steered")]
    InputSteered(InputSteered),
    /// `thread.switched`
    #[serde(rename = "thread.switched")]
    ThreadSwitched(ThreadSwitched),
    /// `handoff.delivered`
    #[serde(rename = "handoff.delivered")]
    HandoffDelivered(HandoffDelivered),
}

impl ThreadEvent {
    /// Every kind string, in spec table order.
    pub const KINDS: [&'static str; 22] = [
        "turn.started",
        "text.delta",
        "text.done",
        "reasoning.delta",
        "tool.started",
        "tool.output",
        "tool.finished",
        "file.changed",
        "approval.requested",
        "approval.resolved",
        "usage",
        "turn.ended",
        "plugin.event",
        "queue.added",
        "queue.updated",
        "queue.removed",
        "queue.moved",
        "queue.held",
        "queue.resumed",
        "input.steered",
        "thread.switched",
        "handoff.delivered",
    ];

    /// This event's kind string.
    pub fn kind(&self) -> &'static str {
        match self {
            ThreadEvent::TurnStarted(_) => "turn.started",
            ThreadEvent::TextDelta(_) => "text.delta",
            ThreadEvent::TextDone(_) => "text.done",
            ThreadEvent::ReasoningDelta(_) => "reasoning.delta",
            ThreadEvent::ToolStarted(_) => "tool.started",
            ThreadEvent::ToolOutput(_) => "tool.output",
            ThreadEvent::ToolFinished(_) => "tool.finished",
            ThreadEvent::FileChanged(_) => "file.changed",
            ThreadEvent::ApprovalRequested(_) => "approval.requested",
            ThreadEvent::ApprovalResolved(_) => "approval.resolved",
            ThreadEvent::Usage(_) => "usage",
            ThreadEvent::TurnEnded(_) => "turn.ended",
            ThreadEvent::PluginEvent(_) => "plugin.event",
            ThreadEvent::QueueAdded(_) => "queue.added",
            ThreadEvent::QueueUpdated(_) => "queue.updated",
            ThreadEvent::QueueRemoved(_) => "queue.removed",
            ThreadEvent::QueueMoved(_) => "queue.moved",
            ThreadEvent::QueueHeld(_) => "queue.held",
            ThreadEvent::QueueResumed(_) => "queue.resumed",
            ThreadEvent::InputSteered(_) => "input.steered",
            ThreadEvent::ThreadSwitched(_) => "thread.switched",
            ThreadEvent::HandoffDelivered(_) => "handoff.delivered",
        }
    }

    /// The turn this event belongs to, if it names one directly.
    pub fn turn(&self) -> Option<&TurnId> {
        match self {
            ThreadEvent::TurnStarted(e) => Some(&e.turn),
            ThreadEvent::TextDelta(e)
            | ThreadEvent::TextDone(e)
            | ThreadEvent::ReasoningDelta(e) => Some(&e.turn),
            ThreadEvent::ToolStarted(e) => Some(&e.turn),
            ThreadEvent::ToolOutput(e) => Some(&e.turn),
            ThreadEvent::ToolFinished(e) => Some(&e.turn),
            ThreadEvent::FileChanged(e) => Some(&e.turn),
            ThreadEvent::ApprovalRequested(e) => Some(&e.approval.turn),
            ThreadEvent::Usage(e) => Some(&e.turn),
            ThreadEvent::TurnEnded(e) => Some(&e.turn),
            ThreadEvent::InputSteered(e) => Some(&e.turn),
            ThreadEvent::HandoffDelivered(e) => Some(&e.turn),
            ThreadEvent::ApprovalResolved(_)
            | ThreadEvent::PluginEvent(_)
            | ThreadEvent::QueueAdded(_)
            | ThreadEvent::QueueUpdated(_)
            | ThreadEvent::QueueRemoved(_)
            | ThreadEvent::QueueMoved(_)
            | ThreadEvent::QueueHeld(_)
            | ThreadEvent::QueueResumed(_)
            | ThreadEvent::ThreadSwitched(_) => None,
        }
    }
}

/// Data of a workspace event: the thread as it is now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadChange {
    /// The thread after the change.
    pub thread: Thread,
}

/// An event on a `workspace:<id>` topic: `kind` plus `data`. Not logged; only live
/// changes are delivered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data")]
pub enum WorkspaceEvent {
    /// `thread.created`
    #[serde(rename = "thread.created")]
    ThreadCreated(ThreadChange),
    /// `thread.updated`: the thread's status or title changed.
    #[serde(rename = "thread.updated")]
    ThreadUpdated(ThreadChange),
    /// `thread.archived`
    #[serde(rename = "thread.archived")]
    ThreadArchived(ThreadChange),
    /// `thread.deleted`: the thread was permanently removed.
    #[serde(rename = "thread.deleted")]
    ThreadDeleted(ThreadChange),
}

impl WorkspaceEvent {
    /// Every kind string, in spec table order.
    pub const KINDS: [&'static str; 4] = [
        "thread.created",
        "thread.updated",
        "thread.archived",
        "thread.deleted",
    ];

    /// This event's kind string.
    pub fn kind(&self) -> &'static str {
        match self {
            WorkspaceEvent::ThreadCreated(_) => "thread.created",
            WorkspaceEvent::ThreadUpdated(_) => "thread.updated",
            WorkspaceEvent::ThreadArchived(_) => "thread.archived",
            WorkspaceEvent::ThreadDeleted(_) => "thread.deleted",
        }
    }

    /// The thread it is about.
    pub fn thread(&self) -> &Thread {
        match self {
            WorkspaceEvent::ThreadCreated(c)
            | WorkspaceEvent::ThreadUpdated(c)
            | WorkspaceEvent::ThreadArchived(c)
            | WorkspaceEvent::ThreadDeleted(c) => &c.thread,
        }
    }
}

/// An event on any topic. Kinds never overlap, so the `kind` string says which.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum TopicEvent {
    /// An event of a thread's log.
    Thread(ThreadEvent),
    /// A change in a workspace's threads.
    Workspace(WorkspaceEvent),
}

impl From<ThreadEvent> for TopicEvent {
    fn from(e: ThreadEvent) -> Self {
        TopicEvent::Thread(e)
    }
}

impl From<WorkspaceEvent> for TopicEvent {
    fn from(e: WorkspaceEvent) -> Self {
        TopicEvent::Workspace(e)
    }
}

/// Params of the `events.event` notification: one event on a topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventNotification {
    /// The topic, for example `thread:01J…`.
    pub topic: String,
    /// Sequence number, starting at 1: within the thread for thread topics (a cursor),
    /// within the subscription for workspace topics (not a cursor).
    pub seq: u64,
    /// When the kernel appended it, or when the change happened.
    pub time: Timestamp,
    /// Kind and data.
    #[serde(flatten)]
    pub event: TopicEvent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_is_kind_and_data() {
        let e = ThreadEvent::TextDelta(TurnText {
            turn: "t1".into(),
            text: "hi".into(),
        });
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"kind":"text.delta","data":{"turn":"t1","text":"hi"}})
        );
    }

    #[test]
    fn notification_flattens_the_event() {
        let n = EventNotification {
            topic: "thread:x".into(),
            seq: 3,
            time: Timestamp("2026-11-15T10:00:00.000Z".into()),
            event: ThreadEvent::Usage(Usage {
                turn: "t1".into(),
                input_tokens: 10,
                output_tokens: 2,
                cost: None,
            })
            .into(),
        };
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(
            v,
            json!({"topic":"thread:x","seq":3,"time":"2026-11-15T10:00:00.000Z",
                   "kind":"usage","data":{"turn":"t1","inputTokens":10,"outputTokens":2}})
        );
        assert_eq!(serde_json::from_value::<EventNotification>(v).unwrap(), n);
    }

    #[test]
    fn kind_strings_match_serde() {
        let e = ThreadEvent::TurnEnded(TurnEnded {
            turn: "t".into(),
            outcome: TurnOutcome::Failed,
            error: Some("x".into()),
        });
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], e.kind());
        assert!(ThreadEvent::KINDS.contains(&e.kind()));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let e: ThreadEvent = serde_json::from_value(
            json!({"kind":"text.delta","data":{"turn":"t1","text":"hi","future":1}}),
        )
        .unwrap();
        assert_eq!(e.kind(), "text.delta");
    }
}
