//! The method table: every kernel method with its params and result types, declared once.
//!
//! The core dispatches with it, TypeScript generation walks it, and the fixture tests use
//! it to check that every method has a recorded example.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use ts_rs::TS;

use crate::event::EventNotification;
use crate::model::{
    Approval, ApprovalId, Decision, Thread, ThreadId, Turn, TurnId, UserInput, Workspace,
    WorkspaceId,
};

/// A request method: name, params and result.
pub trait Method {
    /// The wire name, for example `thread.create`.
    const NAME: &'static str;
    /// Params type. Absent params are read as `{}`.
    type Params: Serialize + DeserializeOwned + TS + 'static;
    /// Result type.
    type Result: Serialize + DeserializeOwned + TS + 'static;
}

/// A notification the core sends: name and params.
pub trait NotificationMethod {
    /// The wire name, for example `events.event`.
    const NAME: &'static str;
    /// Params type.
    type Params: Serialize + DeserializeOwned + TS + 'static;
}

/// Walks the method table; see [`visit_all`].
pub trait MethodVisitor {
    /// Called once per request method.
    fn method<M: Method>(&mut self);
    /// Called once per notification.
    fn notification<N: NotificationMethod>(&mut self);
}

macro_rules! methods {
    (
        methods { $( $(#[$doc:meta])* $ty:ident = $name:literal ($params:ty) -> $result:ty; )* }
        notifications { $( $(#[$ndoc:meta])* $nty:ident = $nname:literal ($nparams:ty); )* }
    ) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, Copy)]
            pub struct $ty;
            impl Method for $ty {
                const NAME: &'static str = $name;
                type Params = $params;
                type Result = $result;
            }
        )*
        $(
            $(#[$ndoc])*
            #[derive(Debug, Clone, Copy)]
            pub struct $nty;
            impl NotificationMethod for $nty {
                const NAME: &'static str = $nname;
                type Params = $nparams;
            }
        )*

        /// Every request method name, in table order.
        pub const METHOD_NAMES: &[&str] = &[$($name),*];
        /// Every notification name, in table order.
        pub const NOTIFICATION_NAMES: &[&str] = &[$($nname),*];

        /// Calls the visitor for every method and notification, in table order.
        pub fn visit_all(visitor: &mut impl MethodVisitor) {
            $( visitor.method::<$ty>(); )*
            $( visitor.notification::<$nty>(); )*
        }
    };
}

methods! {
    methods {
        /// `core.hello`: the handshake; the first call on a connection.
        CoreHello = "core.hello" (HelloParams) -> HelloResult;
        /// `workspace.list`: open workspaces.
        WorkspaceList = "workspace.list" (Empty) -> WorkspaceListResult;
        /// `workspace.open`: register a directory, or return the existing workspace for it.
        WorkspaceOpen = "workspace.open" (WorkspaceOpenParams) -> WorkspaceOpenResult;
        /// `workspace.close`: hide a workspace; its data is kept.
        WorkspaceClose = "workspace.close" (WorkspaceRef) -> Empty;
        /// `thread.list`: threads of a workspace.
        ThreadList = "thread.list" (ThreadListParams) -> ThreadListResult;
        /// `thread.create`: a new thread.
        ThreadCreate = "thread.create" (ThreadCreateParams) -> ThreadCreateResult;
        /// `thread.get`: a snapshot folded from the log.
        ThreadGet = "thread.get" (ThreadRef) -> ThreadSnapshot;
        /// `thread.send`: a user message; starts or queues a turn.
        ThreadSend = "thread.send" (ThreadSendParams) -> ThreadSendResult;
        /// `thread.cancel`: cancel the running turn and drop queued messages.
        ThreadCancel = "thread.cancel" (ThreadRef) -> Empty;
        /// `thread.respondApproval`: answer a pending approval; the first answer wins.
        ThreadRespondApproval = "thread.respondApproval" (RespondApprovalParams) -> Empty;
        /// `thread.archive`: archive a thread.
        ThreadArchive = "thread.archive" (ThreadRef) -> Empty;
        /// `events.subscribe`: replay a topic after a cursor, then follow it live.
        EventsSubscribe = "events.subscribe" (SubscribeParams) -> SubscribeResult;
        /// `events.unsubscribe`: stop following a topic.
        EventsUnsubscribe = "events.unsubscribe" (UnsubscribeParams) -> Empty;
    }
    notifications {
        /// `events.event`: one event on a subscribed topic.
        EventsEvent = "events.event" (EventNotification);
    }
}

/// An empty object, `{}`, used for methods without params or result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Empty {}

/// Name and version of a client or core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    /// Program name, for example `kumikit-desktop`.
    pub name: String,
    /// Program version.
    pub version: String,
}

/// Params of `core.hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HelloParams {
    /// The client's protocol version, `MAJOR.MINOR`.
    pub protocol: String,
    /// Who is connecting.
    pub client: ClientInfo,
}

/// The core's description of itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CoreInfo {
    /// Core version.
    pub version: String,
    /// Display name, usually the host name.
    pub name: String,
    /// Operating system: `linux`, `macos` or `windows`.
    pub os: String,
}

/// What a plugin adds to the core, as far as clients need to know.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Contributions {
    /// Provider ids.
    #[serde(default)]
    pub providers: Vec<String>,
    /// Command ids.
    #[serde(default)]
    pub commands: Vec<String>,
    /// Whether the plugin has settings.
    #[serde(default)]
    pub settings: bool,
}

/// A plugin installed on the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    /// Plugin id.
    pub id: String,
    /// Plugin version.
    pub version: String,
    /// What it contributes.
    pub contributes: Contributions,
}

/// Result of `core.hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HelloResult {
    /// The protocol version the core speaks.
    pub protocol: String,
    /// The core.
    pub core: CoreInfo,
    /// Installed plugins.
    pub plugins: Vec<PluginInfo>,
    /// Feature strings; a client shows no UI for features not listed.
    pub capabilities: Vec<String>,
}

/// Result of `workspace.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceListResult {
    /// Open workspaces.
    pub workspaces: Vec<Workspace>,
}

/// Params of `workspace.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceOpenParams {
    /// Absolute path of an existing directory on the core's machine.
    pub path: String,
}

/// Result of `workspace.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceOpenResult {
    /// The workspace.
    pub workspace: Workspace,
}

/// Params naming one workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRef {
    /// The workspace.
    pub workspace: WorkspaceId,
}

/// Params of `thread.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ThreadListParams {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// Include archived threads (default false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_archived: Option<bool>,
}

/// Result of `thread.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListResult {
    /// Threads, most recently updated first.
    pub threads: Vec<Thread>,
}

/// Params of `thread.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ThreadCreateParams {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// Provider id; defaults to the only provider when exactly one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model id; defaults to the provider's first model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Result of `thread.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCreateResult {
    /// The new thread.
    pub thread: Thread,
}

/// Params naming one thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRef {
    /// The thread.
    pub thread: ThreadId,
}

/// Result of `thread.get`: state folded from the log up to `seq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSnapshot {
    /// The thread.
    pub thread: Thread,
    /// Its turns, oldest first.
    pub turns: Vec<Turn>,
    /// Approvals still waiting for an answer.
    pub pending_approvals: Vec<Approval>,
    /// The last event this snapshot includes; subscribe with `after: seq` to follow on.
    pub seq: u64,
}

/// Params of `thread.send`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSendParams {
    /// The thread.
    pub thread: ThreadId,
    /// The message.
    pub input: UserInput,
}

/// Result of `thread.send`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSendResult {
    /// The turn this message will run as.
    pub turn: TurnId,
    /// True if another turn is running and this one waits in the queue.
    pub queued: bool,
}

/// Params of `thread.respondApproval`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RespondApprovalParams {
    /// The approval.
    pub approval: ApprovalId,
    /// The answer.
    pub decision: Decision,
}

/// Params of `events.subscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeParams {
    /// The topic, for example `thread:01J…`.
    pub topic: String,
    /// Replay events with `seq > after`; 0 replays from the start.
    #[serde(default)]
    pub after: u64,
}

/// Result of `events.subscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeResult {
    /// The topic's last `seq` when the subscription started.
    pub head: u64,
}

/// Params of `events.unsubscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnsubscribeParams {
    /// The topic.
    pub topic: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        let mut all: Vec<&str> = METHOD_NAMES
            .iter()
            .chain(NOTIFICATION_NAMES)
            .copied()
            .collect();
        all.sort_unstable();
        let before = all.len();
        all.dedup();
        assert_eq!(all.len(), before);
    }

    #[test]
    fn visitor_sees_every_method() {
        struct Names(Vec<&'static str>);
        impl MethodVisitor for Names {
            fn method<M: Method>(&mut self) {
                self.0.push(M::NAME);
            }
            fn notification<N: NotificationMethod>(&mut self) {
                self.0.push(N::NAME);
            }
        }
        let mut v = Names(Vec::new());
        visit_all(&mut v);
        assert_eq!(v.0.len(), METHOD_NAMES.len() + NOTIFICATION_NAMES.len());
    }

    #[test]
    fn empty_accepts_unknown_fields() {
        let e: Empty = serde_json::from_str(r#"{"x":1}"#).unwrap();
        assert_eq!(serde_json::to_string(&e).unwrap(), "{}");
    }

    #[test]
    fn subscribe_after_defaults_to_zero() {
        let p: SubscribeParams = serde_json::from_str(r#"{"topic":"thread:a"}"#).unwrap();
        assert_eq!(p.after, 0);
    }
}
