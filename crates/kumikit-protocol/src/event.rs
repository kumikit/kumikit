//! Thread events (`docs/spec/03-kernel.md#thread-events`), workspace events
//! (`docs/spec/02-protocol.md#workspace-topic`) and the `events.event` notification that
//! carries them.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::model::{
    Approval, ApprovalId, Resolution, Thread, Timestamp, TurnId, TurnOutcome, UserInput,
};

/// `turn.started`: the kernel started a turn for a user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TurnStarted {
    /// The new turn.
    pub turn: TurnId,
    /// The user message that started it.
    pub input: UserInput,
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
}

impl ThreadEvent {
    /// Every kind string, in spec table order.
    pub const KINDS: [&'static str; 13] = [
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
            ThreadEvent::ApprovalResolved(_) | ThreadEvent::PluginEvent(_) => None,
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
}

impl WorkspaceEvent {
    /// Every kind string, in spec table order.
    pub const KINDS: [&'static str; 3] = ["thread.created", "thread.updated", "thread.archived"];

    /// This event's kind string.
    pub fn kind(&self) -> &'static str {
        match self {
            WorkspaceEvent::ThreadCreated(_) => "thread.created",
            WorkspaceEvent::ThreadUpdated(_) => "thread.updated",
            WorkspaceEvent::ThreadArchived(_) => "thread.archived",
        }
    }

    /// The thread it is about.
    pub fn thread(&self) -> &Thread {
        match self {
            WorkspaceEvent::ThreadCreated(c)
            | WorkspaceEvent::ThreadUpdated(c)
            | WorkspaceEvent::ThreadArchived(c) => &c.thread,
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
