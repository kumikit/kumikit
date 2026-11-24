//! Domain types shared by the kernel and clients (`docs/spec/03-kernel.md#domain-model`).

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// On the wire a ULID string. This crate does not validate the format; the kernel
        /// generates ids and looks up the ones it receives.
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// The id as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                $name(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                $name(value.to_owned())
            }
        }
    };
}

id_type!(
    /// Identifies a workspace.
    WorkspaceId
);
id_type!(
    /// Identifies a thread.
    ThreadId
);
id_type!(
    /// Identifies a turn within a thread.
    TurnId
);
id_type!(
    /// Identifies an approval request.
    ApprovalId
);
id_type!(
    /// Identifies a message waiting in a thread's queue.
    QueueEntryId
);

/// An RFC 3339 UTC timestamp, for example `2026-11-15T10:00:00.000Z`. Assigned by the
/// kernel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct Timestamp(pub String);

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether a workspace represents a user project folder (tasks) or the core's
/// folderless chat space (chats).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    /// A user-chosen project folder; threads in it are tasks.
    #[default]
    Project,
    /// Core-managed scratch directory for chats that do not require a project folder.
    Chat,
}

impl WorkspaceKind {
    /// The wire/storage string.
    pub fn as_str(self) -> &'static str {
        match self {
            WorkspaceKind::Project => "project",
            WorkspaceKind::Chat => "chat",
        }
    }

    /// Parses the wire/storage string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "project" => Some(WorkspaceKind::Project),
            "chat" => Some(WorkspaceKind::Chat),
            _ => None,
        }
    }
}

/// A project directory (or core-managed chat workspace) known to a core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    /// Workspace id.
    pub id: WorkspaceId,
    /// Display name; the root directory's last component, or `Chats` for the chat workspace.
    pub name: String,
    /// Canonical absolute path of the root directory.
    pub root: String,
    /// Workspace kind (`project` by default; `chat` for folderless chats). Omitted when `project`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kind: Option<WorkspaceKind>,
}

/// Where a thread is in its lifecycle. Derived from the log, except `archived`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    /// No turn is running.
    Idle,
    /// A turn is running.
    Running,
    /// A turn waits for an approval.
    AwaitingApproval,
    /// The last turn failed.
    Failed,
    /// Archived; hidden from `thread.list` by default.
    Archived,
}

impl ThreadStatus {
    /// The wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadStatus::Idle => "idle",
            ThreadStatus::Running => "running",
            ThreadStatus::AwaitingApproval => "awaiting_approval",
            ThreadStatus::Failed => "failed",
            ThreadStatus::Archived => "archived",
        }
    }

    /// Parses the wire string.
    pub fn parse(value: &str) -> Option<Self> {
        [
            ThreadStatus::Idle,
            ThreadStatus::Running,
            ThreadStatus::AwaitingApproval,
            ThreadStatus::Failed,
            ThreadStatus::Archived,
        ]
        .into_iter()
        .find(|s| s.as_str() == value)
    }
}

/// How aggressively an agent provider asks for approval before acting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Ask before edits and commands (default).
    #[default]
    Standard,
    /// Approve file edits automatically; ask before running commands.
    AcceptEdits,
    /// Read-only planning and exploration; do not edit files or run mutating commands.
    Plan,
    /// Allow edits and commands without asking.
    FullAccess,
}

impl PermissionMode {
    /// The wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            PermissionMode::Standard => "standard",
            PermissionMode::AcceptEdits => "accept_edits",
            PermissionMode::Plan => "plan",
            PermissionMode::FullAccess => "full_access",
        }
    }

    /// Parses the wire string.
    pub fn parse(value: &str) -> Option<Self> {
        [
            PermissionMode::Standard,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::FullAccess,
        ]
        .into_iter()
        .find(|m| m.as_str() == value)
    }
}

/// A conversation with an agent inside a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Thread {
    /// Thread id.
    pub id: ThreadId,
    /// The workspace it belongs to.
    pub workspace: WorkspaceId,
    /// Optional title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Provider id.
    pub provider: String,
    /// Model id, as reported by the provider.
    pub model: String,
    /// Permission mode for the agent session; absent means `standard`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// Linked Git worktree directory this thread runs in; absent means the workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    /// Current status.
    pub status: ThreadStatus,
    /// When it was created.
    pub created: Timestamp,
    /// When its log or status last changed.
    pub updated: Timestamp,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    /// The provider finished the turn.
    Done,
    /// The turn was cancelled.
    Cancelled,
    /// The provider or the core failed.
    Failed,
}

impl TurnOutcome {
    /// The wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            TurnOutcome::Done => "done",
            TurnOutcome::Cancelled => "cancelled",
            TurnOutcome::Failed => "failed",
        }
    }

    /// Parses the wire string.
    pub fn parse(value: &str) -> Option<Self> {
        [
            TurnOutcome::Done,
            TurnOutcome::Cancelled,
            TurnOutcome::Failed,
        ]
        .into_iter()
        .find(|s| s.as_str() == value)
    }
}

/// One user message and the agent's work on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Turn {
    /// Turn id.
    pub id: TurnId,
    /// The thread it belongs to.
    pub thread: ThreadId,
    /// When it started.
    pub started: Timestamp,
    /// When it ended; absent while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<Timestamp>,
    /// How it ended; absent while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TurnOutcome>,
}

/// Where an approval is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    /// Waiting for an answer.
    Pending,
    /// Answered with `allow` or `allow_always`.
    Allowed,
    /// Answered with `deny`.
    Denied,
    /// The turn ended before anyone answered.
    Cancelled,
}

/// An answer to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Allow this once.
    Allow,
    /// Allow this and similar requests from now on (the provider decides the scope).
    AllowAlways,
    /// Deny.
    Deny,
}

/// How an approval was resolved: a [`Decision`], or cancelled by the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Allowed once.
    Allow,
    /// Allowed from now on.
    AllowAlways,
    /// Denied.
    Deny,
    /// The turn ended before anyone answered.
    Cancelled,
}

impl From<Decision> for Resolution {
    fn from(d: Decision) -> Self {
        match d {
            Decision::Allow => Resolution::Allow,
            Decision::AllowAlways => Resolution::AllowAlways,
            Decision::Deny => Resolution::Deny,
        }
    }
}

impl Resolution {
    /// The approval status this resolution leads to.
    pub fn status(self) -> ApprovalStatus {
        match self {
            Resolution::Allow | Resolution::AllowAlways => ApprovalStatus::Allowed,
            Resolution::Deny => ApprovalStatus::Denied,
            Resolution::Cancelled => ApprovalStatus::Cancelled,
        }
    }
}

/// A request from the agent that a person (or bot) must answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Approval {
    /// Approval id, assigned by the kernel.
    pub id: ApprovalId,
    /// The thread it belongs to.
    pub thread: ThreadId,
    /// The turn that asked.
    pub turn: TurnId,
    /// What is being approved, for example `tool`. Free-form, chosen by the provider.
    pub kind: String,
    /// One-line description for the person answering.
    pub summary: String,
    /// Structured detail, for example the tool input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
    /// Current status.
    pub status: ApprovalStatus,
}

/// A file or image attached to a user message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Attachment {
    /// Display filename, for example `Pasted image.png`.
    pub name: String,
    /// MIME type, for example `image/png` or `text/plain`.
    pub mime_type: String,
    /// Base64-encoded bytes (standard alphabet).
    pub data: String,
    /// Image width in pixels, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Image height in pixels, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// A user message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UserInput {
    /// The message text.
    pub text: String,
    /// Optional files or images attached to the message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<Attachment>>", optional)]
    pub attachments: Vec<Attachment>,
}

impl UserInput {
    /// A plain text user message with no attachments.
    pub fn text(text: impl Into<String>) -> Self {
        UserInput {
            text: text.into(),
            attachments: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn thread_shape() {
        let t = Thread {
            id: "th".into(),
            workspace: "ws".into(),
            title: None,
            provider: "p".into(),
            model: "m".into(),
            permission_mode: None,
            worktree_path: None,
            status: ThreadStatus::AwaitingApproval,
            created: Timestamp("t0".into()),
            updated: Timestamp("t1".into()),
        };
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            json!({"id":"th","workspace":"ws","provider":"p","model":"m",
                   "status":"awaiting_approval","created":"t0","updated":"t1"})
        );
    }

    #[test]
    fn permission_mode_round_trip() {
        for s in ["standard", "accept_edits", "plan", "full_access"] {
            let parsed = PermissionMode::parse(s).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), s);
        }
    }

    #[test]
    fn enum_values_are_snake_case() {
        assert_eq!(
            serde_json::to_value(Decision::AllowAlways).unwrap(),
            "allow_always"
        );
        assert_eq!(
            serde_json::to_value(Resolution::Cancelled).unwrap(),
            "cancelled"
        );
        assert_eq!(serde_json::to_value(TurnOutcome::Done).unwrap(), "done");
    }

    #[test]
    fn status_strings_match_serde() {
        for s in ["idle", "running", "awaiting_approval", "failed", "archived"] {
            let parsed = ThreadStatus::parse(s).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), s);
        }
        for s in ["done", "cancelled", "failed"] {
            let parsed = TurnOutcome::parse(s).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), s);
        }
    }

    #[test]
    fn resolution_status() {
        assert_eq!(
            Resolution::from(Decision::AllowAlways).status(),
            ApprovalStatus::Allowed
        );
        assert_eq!(Resolution::Deny.status(), ApprovalStatus::Denied);
    }
}
