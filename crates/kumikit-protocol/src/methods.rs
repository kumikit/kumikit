//! The method table: every kernel method with its params and result types, declared once.
//!
//! The core dispatches with it, TypeScript generation walks it, and the fixture tests use
//! it to check that every method has a recorded example.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use ts_rs::TS;

use crate::event::{EventNotification, QueueEntry, QueueHold, QueueTrigger};
use crate::model::{
    Approval, ApprovalId, Decision, PermissionMode, QueueEntryId, Thread, ThreadId, Timestamp,
    Turn, TurnId, UserInput, Workspace, WorkspaceId,
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
        /// `auth.pair`: pair a new device with a one-time code; a remote connection's first
        /// message.
        AuthPair = "auth.pair" (AuthPairParams) -> AuthPairResult;
        /// `auth.token`: authenticate a remote connection as a paired device; its first
        /// message.
        AuthToken = "auth.token" (AuthTokenParams) -> AuthTokenResult;
        /// `auth.pairing.create`: a new pairing code; local connections only.
        AuthPairingCreate = "auth.pairing.create" (Empty) -> PairingCode;
        /// `auth.devices.list`: paired devices.
        AuthDevicesList = "auth.devices.list" (Empty) -> DeviceListResult;
        /// `auth.devices.revoke`: unpair a device and close its connections.
        AuthDevicesRevoke = "auth.devices.revoke" (DeviceRef) -> Empty;
        /// `core.hello`: the handshake; the first call on a connection.
        CoreHello = "core.hello" (HelloParams) -> HelloResult;
        /// `core.config.get`: reads core-level configuration (such as the Kumikit home folder).
        CoreConfigGet = "core.config.get" (Empty) -> CoreConfigResult;
        /// `core.config.set`: updates core-level configuration (omitting or clearing resets to default).
        CoreConfigSet = "core.config.set" (CoreConfigSetParams) -> CoreConfigResult;
        /// `core.worktrees.list`: lists managed Git worktrees under `<homeDir>/worktrees/` with disk usage.
        CoreWorktreesList = "core.worktrees.list" (Empty) -> WorktreeListResult;
        /// `core.worktrees.delete`: removes a managed Git worktree directory under `<homeDir>/worktrees/`.
        CoreWorktreesDelete = "core.worktrees.delete" (WorktreeDeleteParams) -> Empty;
        /// `core.activity`: summarises live agent turns and ready queued messages across
        /// all workspaces so updaters can wait for idle before stopping the core (since 0.10).
        CoreActivity = "core.activity" (Empty) -> CoreActivityResult;
        /// `core.shutdown`: stop the core; with `ifIdle: true`, refuses while agents are busy.
        CoreShutdown = "core.shutdown" (ShutdownParams) -> Empty;
        /// `provider.list`: available providers, from manifests, without activating any.
        ProviderList = "provider.list" (Empty) -> ProviderListResult;
        /// `provider.models`: a provider's models; activates its plugin.
        ProviderModels = "provider.models" (ProviderModelsParams) -> ProviderModelsResult;
        /// `workspace.list`: open workspaces.
        WorkspaceList = "workspace.list" (Empty) -> WorkspaceListResult;
        /// `workspace.open`: register a directory, or return the existing workspace for it.
        WorkspaceOpen = "workspace.open" (WorkspaceOpenParams) -> WorkspaceOpenResult;
        /// `workspace.chat`: open or create the core-managed chat workspace (no project folder selection required).
        WorkspaceChat = "workspace.chat" (Empty) -> WorkspaceOpenResult;
        /// `workspace.close`: hide a workspace; its data is kept.
        WorkspaceClose = "workspace.close" (WorkspaceRef) -> Empty;
        /// `workspace.delete`: permanently delete a workspace and all its threads and history.
        WorkspaceDelete = "workspace.delete" (WorkspaceRef) -> Empty;
        /// `workspace.git.status`: inspects a workspace's (or worktree's) Git branch and local branches.
        WorkspaceGitStatus = "workspace.git.status" (WorkspaceGitStatusParams) -> WorkspaceGitInfo;
        /// `workspace.git.checkout`: switches the workspace (or worktree) to an existing local Git branch.
        WorkspaceGitCheckout = "workspace.git.checkout" (WorkspaceGitCheckoutParams) -> WorkspaceGitInfo;
        /// `workspace.git.worktreeCreate`: creates a Git worktree under `<homeDir>/worktrees/` for a thread in this workspace.
        WorkspaceGitWorktreeCreate = "workspace.git.worktreeCreate" (WorkspaceWorktreeCreateParams) -> WorkspaceWorktreeCreateResult;
        /// `workspace.files.list`: lists workspace-relative file paths for `@` mentions (since 0.9).
        WorkspaceFilesList = "workspace.files.list" (WorkspaceFilesListParams) -> WorkspaceFilesListResult;
        /// `thread.list`: threads of a workspace.
        ThreadList = "thread.list" (ThreadListParams) -> ThreadListResult;
        /// `thread.create`: a new thread.
        ThreadCreate = "thread.create" (ThreadCreateParams) -> ThreadCreateResult;
        /// `thread.fork`: copy a thread's history up to a turn into a new thread (since 0.9).
        ThreadFork = "thread.fork" (ThreadForkParams) -> ThreadCreateResult;
        /// `thread.update`: updates a thread's model, permission mode, or title.
        ThreadUpdate = "thread.update" (ThreadUpdateParams) -> ThreadUpdateResult;
        /// `thread.get`: a snapshot folded from the log.
        ThreadGet = "thread.get" (ThreadRef) -> ThreadSnapshot;
        /// `thread.send`: a user message; starts or queues a turn.
        ThreadSend = "thread.send" (ThreadSendParams) -> ThreadSendResult;
        /// `thread.cancel`: cancel the running turn; queued messages are held.
        ThreadCancel = "thread.cancel" (ThreadRef) -> Empty;
        /// `thread.steer`: a message for the running turn's agent (since 0.8).
        ThreadSteer = "thread.steer" (ThreadSendParams) -> ThreadSteerResult;
        /// `thread.defer`: queue a message that waits for a trigger before running (since 0.9).
        ThreadDefer = "thread.defer" (ThreadDeferParams) -> QueueEntryResult;
        /// `thread.queue.update`: replace a queued message's input.
        ThreadQueueUpdate = "thread.queue.update" (QueueUpdateParams) -> QueueEntryResult;
        /// `thread.queue.remove`: remove a queued message.
        ThreadQueueRemove = "thread.queue.remove" (QueueEntryRef) -> Empty;
        /// `thread.queue.move`: move a queued message before another, or to the end.
        ThreadQueueMove = "thread.queue.move" (QueueMoveParams) -> Empty;
        /// `thread.queue.sendNow`: start a queued message now, stopping the running turn.
        ThreadQueueSendNow = "thread.queue.sendNow" (QueueEntryRef) -> ThreadSendResult;
        /// `thread.queue.resume`: let a held queue start its entries again.
        ThreadQueueResume = "thread.queue.resume" (ThreadRef) -> Empty;
        /// `thread.respondApproval`: answer a pending approval; the first answer wins.
        ThreadRespondApproval = "thread.respondApproval" (RespondApprovalParams) -> Empty;
        /// `thread.archive`: archive a thread.
        ThreadArchive = "thread.archive" (ThreadRef) -> Empty;
        /// `thread.unarchive`: restore an archived thread back to its workspace.
        ThreadUnarchive = "thread.unarchive" (ThreadRef) -> ThreadUpdateResult;
        /// `thread.delete`: permanently delete a thread and its history.
        ThreadDelete = "thread.delete" (ThreadRef) -> Empty;
        /// `events.subscribe`: replay a topic after a cursor, then follow it live.
        EventsSubscribe = "events.subscribe" (SubscribeParams) -> SubscribeResult;
        /// `events.unsubscribe`: stop following a topic.
        EventsUnsubscribe = "events.unsubscribe" (UnsubscribeParams) -> Empty;
        /// `plugin.list`: every installed core plugin.
        PluginList = "plugin.list" (Empty) -> PluginListResult;
        /// `plugin.install`: install a `.kkp` file from the core's machine; nothing is
        /// granted or enabled.
        PluginInstall = "plugin.install" (PluginInstallParams) -> PluginResult;
        /// `plugin.enable`: grant a plugin's permissions (as a whole) and enable it.
        PluginEnable = "plugin.enable" (PluginEnableParams) -> PluginResult;
        /// `plugin.disable`: disable a plugin; its running turns are cancelled.
        PluginDisable = "plugin.disable" (PluginRef) -> PluginResult;
        /// `plugin.uninstall`: remove a plugin, and its data if asked.
        PluginUninstall = "plugin.uninstall" (PluginUninstallParams) -> Empty;
        /// `plugin.updates`: the updates the registries offer; applies nothing.
        PluginUpdates = "plugin.updates" (PluginUpdatesParams) -> PluginUpdatesResult;
        /// `plugin.update`: replace an installed plugin with another version.
        PluginUpdate = "plugin.update" (PluginUpdateParams) -> PluginResult;
        /// `plugin.lock`: the text of the core's `plugins.lock`.
        PluginLock = "plugin.lock" (Empty) -> PluginLockResult;
        /// `registry.list`: the core's registries, in resolution order.
        RegistryList = "registry.list" (Empty) -> RegistryListResult;
        /// `registry.add`: append a registry.
        RegistryAdd = "registry.add" (RegistryUrl) -> Empty;
        /// `registry.remove`: remove a registry.
        RegistryRemove = "registry.remove" (RegistryUrl) -> Empty;
        /// `registry.search`: core plugins the registries offer.
        RegistrySearch = "registry.search" (RegistrySearchParams) -> RegistrySearchResult;
        /// `secret.set`: stores a secret; values are never returned.
        SecretSet = "secret.set" (SecretSetParams) -> Empty;
        /// `secret.delete`: removes a secret.
        SecretDelete = "secret.delete" (SecretRef) -> Empty;
        /// `secret.list`: secret names, whether each is set, and who may read it.
        SecretList = "secret.list" (Empty) -> SecretListResult;
        /// `settings.schema`: a plugin's settings schema.
        SettingsSchema = "settings.schema" (PluginRef) -> SettingsSchemaResult;
        /// `settings.get`: a plugin's settings values.
        SettingsGet = "settings.get" (PluginRef) -> SettingsGetResult;
        /// `settings.set`: replaces a plugin's settings, validated against its schema.
        SettingsSet = "settings.set" (SettingsSetParams) -> Empty;
        /// `plugin.workspaces`: the workspaces linked to a plugin.
        PluginWorkspaces = "plugin.workspaces" (PluginRef) -> PluginWorkspacesResult;
        /// `plugin.setWorkspaces`: replaces the workspaces linked to a plugin.
        PluginSetWorkspaces = "plugin.setWorkspaces" (PluginSetWorkspacesParams) -> Empty;
        /// `schedule.list`: every schedule on the core.
        ScheduleList = "schedule.list" (Empty) -> ScheduleListResult;
        /// `schedule.setEnabled`: pauses or resumes a schedule.
        ScheduleSetEnabled = "schedule.setEnabled" (ScheduleSetEnabledParams) -> Empty;
        /// `schedule.delete`: removes a plugin's own schedule.
        ScheduleDelete = "schedule.delete" (ScheduleRef) -> Empty;
        /// `schedule.run`: runs a schedule now.
        ScheduleRun = "schedule.run" (ScheduleRef) -> Empty;
        /// `gateway.attach`: turns a local connection into the tool gateway for one
        /// provider session (its first message).
        GatewayAttach = "gateway.attach" (GatewayAttachParams) -> Empty;
        /// `command.list`: the commands of enabled core plugins.
        CommandList = "command.list" (Empty) -> CommandListResult;
        /// `command.run`: runs a core plugin's command.
        CommandRun = "command.run" (CommandRunParams) -> CommandResult;
    }
    notifications {
        /// `events.event`: one event on a subscribed topic.
        EventsEvent = "events.event" (EventNotification);
        /// `core.pluginsChanged`: the enabled plugins or their available contributions
        /// changed; the full new list.
        CorePluginsChanged = "core.pluginsChanged" (PluginsChanged);
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

/// Result of `core.activity`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CoreActivityResult {
    /// Turns currently running or awaiting approval across all workspaces (including side
    /// threads and scheduled turns).
    pub active_turns: u32,
    /// Unpaused, ready queued messages waiting to start across all workspaces.
    pub ready_queued: u32,
    /// True when no turns are active and no ready queued messages are waiting.
    pub idle: bool,
}

/// Params of `core.shutdown`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ShutdownParams {
    /// When true, fail with `conflict` instead of shutting down if any turn is active or
    /// any unpaused queued message is ready to start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_idle: Option<bool>,
}

/// Params of `auth.pair`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AuthPairParams {
    /// The code `kumikit pair` showed (case, `-` and spaces are ignored).
    pub code: String,
    /// A name for this device, 1–64 characters without control characters.
    pub device_name: String,
}

/// Result of `auth.pair`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AuthPairResult {
    /// The device token. Shown once; the core keeps only its hash.
    pub token: String,
    /// The new device.
    pub device: Device,
}

/// Params of `auth.token`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AuthTokenParams {
    /// The device token from `auth.pair`.
    pub token: String,
}

/// Result of `auth.token`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AuthTokenResult {
    /// The device the connection is now authenticated as.
    pub device: Device,
}

/// A paired device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Device {
    /// Device id.
    pub id: String,
    /// The name it gave when it paired.
    pub name: String,
    /// When it paired.
    pub created: Timestamp,
    /// When it last authenticated, to the minute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<Timestamp>,
    /// Present and true for the caller's own device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<bool>,
}

/// Result of `auth.pairing.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    /// The one-time code, formatted `ABCD-EFGH`.
    pub code: String,
    /// When it stops working.
    pub expires: Timestamp,
    /// The core's certificate fingerprint (`SHA256:…`).
    pub fingerprint: String,
    /// Where clients connect (`wss://host:port/rpc`).
    pub url: String,
}

/// Result of `auth.devices.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeviceListResult {
    /// Paired devices, oldest first.
    pub devices: Vec<Device>,
}

/// Names a paired device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRef {
    /// Device id.
    pub device: String,
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
    /// Provider ids, qualified: `<plugin id>/<local id>`.
    #[serde(default)]
    pub providers: Vec<String>,
    /// Command ids.
    #[serde(default)]
    pub commands: Vec<String>,
    /// Whether the plugin has settings.
    #[serde(default)]
    pub settings: bool,
    /// Plugin method names, called as `<plugin id>/<name>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub methods: Vec<String>,
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

/// Params of `core.pluginsChanged`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginsChanged {
    /// Enabled plugins, as in `core.hello`.
    pub plugins: Vec<PluginInfo>,
}

/// Workspace file access a plugin may have.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum FilesAccess {
    /// None.
    #[default]
    None,
    /// Read files in the workspaces it acts for.
    ReadWorkspace,
    /// Read and write them.
    Workspace,
}

/// Resource limits a plugin asks for above the defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Limits {
    /// Linear memory per instance, MiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
    /// Guest time per call, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_timeout_s: Option<u32>,
}

/// Where a plugin's client operations may act.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacesAccess {
    /// Nowhere.
    #[default]
    None,
    /// The workspaces the user linked to the plugin.
    Linked,
    /// Every workspace.
    All,
}

impl WorkspacesAccess {
    /// Whether it is `none`.
    pub fn is_none(&self) -> bool {
        *self == WorkspacesAccess::None
    }
}

/// Permissions a plugin requests or was granted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Permissions {
    /// Programs it may start. A started program runs with the user's full access.
    #[serde(default)]
    pub spawn: Vec<String>,
    /// Hosts it may connect to.
    #[serde(default)]
    pub network: Vec<String>,
    /// Workspace file access.
    #[serde(default)]
    pub files: FilesAccess,
    /// Secret names it may read.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Whether it may hand tools other plugins contribute to its agent.
    #[serde(default)]
    pub tools: bool,
    /// Client operations it may perform (`threads.send`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub client: Vec<String>,
    /// Where its client operations may act.
    #[serde(default, skip_serializing_if = "WorkspacesAccess::is_none")]
    #[ts(as = "Option<WorkspacesAccess>", optional)]
    pub workspaces: WorkspacesAccess,
    /// Whether it may run on a schedule.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub schedules: bool,
    /// Limits above the defaults; absent when it uses the defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
}

/// Whether the user enabled a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    /// Unpacked; permissions not granted.
    Installed,
    /// Granted and enabled.
    Enabled,
    /// Turned off.
    Disabled,
}

/// The last failure of a plugin (a trap or limit breach).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PluginFailure {
    /// What happened.
    pub message: String,
    /// Seconds until it may start again, while it is in backoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_in_seconds: Option<u64>,
}

/// An installed core plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct InstalledPlugin {
    /// Plugin id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Version.
    pub version: String,
    /// One-line description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Source repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// State.
    pub state: PluginState,
    /// What it asks for.
    pub requested: Permissions,
    /// What was granted, once granted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted: Option<Permissions>,
    /// What it contributes and is available now.
    pub contributes: Contributions,
    /// Whether the package was signed and its signature verified.
    pub signed: bool,
    /// Who signed it, and the registry it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<PublisherInfo>,
    /// Where it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PluginSource>,
    /// The last failure, while it matters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<PluginFailure>,
    /// Why the installed package cannot be loaded, if it cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of `plugin.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginListResult {
    /// Installed plugins, by id.
    pub plugins: Vec<InstalledPlugin>,
}

/// Params of `plugin.install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PluginInstallParams {
    /// Absolute path on the core's machine of a `.kkp` file, or of a directory holding a
    /// `plugin.toml`, which is packed as `kumikit plugin pack` would. Exactly one of `path`
    /// and `package` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The bytes of a `.kkp` file, base64 (standard alphabet, padded), for a core on
    /// another machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// A plugin from the core's registries, which the core downloads and checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryRef>,
}

/// A plugin in a registry, for `plugin.install { registry }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct RegistryRef {
    /// Plugin id.
    pub id: String,
    /// A version; default: the latest this core can run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The SHA-256 the package must have (from a lockfile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// A registry's index URL to use instead of the core's list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

/// Who signed a package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PublisherInfo {
    /// The signing key's id (`ed25519:…`).
    pub key: String,
    /// The registry (index URL) the plugin was installed from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

/// Where an installed plugin came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PluginSource {
    /// A registry.
    Registry,
    /// A package file or folder.
    File,
    /// Preinstalled with the app.
    Bundled,
}

/// Params of `plugin.updates`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PluginUpdatesParams {
    /// Revalidate the registries' indexes now instead of using a recent copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<bool>,
}

/// An update a registry offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    /// Plugin id.
    pub plugin: String,
    /// The installed version.
    pub from: String,
    /// The version offered.
    pub to: String,
    /// The registry offering it.
    pub registry: String,
    /// What the new version asks for.
    pub requested: Permissions,
    /// Whether it asks for more than is granted now; such an update needs a new grant.
    pub widens: bool,
}

/// Result of `plugin.updates`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginUpdatesResult {
    /// Available updates, by plugin id.
    pub updates: Vec<AvailableUpdate>,
}

/// Params of `plugin.update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PluginUpdateParams {
    /// Plugin id.
    pub plugin: String,
    /// The version to install.
    pub version: String,
    /// The new requested permissions, granted as a whole; needed when they widen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<Permissions>,
}

/// Result of `plugin.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginLockResult {
    /// The lockfile's text (TOML).
    pub lock: String,
}

/// A registry by URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RegistryUrl {
    /// The index URL: `https://…`, `http://` on loopback, or `file:`.
    pub url: String,
}

/// A configured registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct RegistryInfo {
    /// The index URL.
    pub url: String,
    /// Its name, once its index was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Result of `registry.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RegistryListResult {
    /// In resolution order.
    pub registries: Vec<RegistryInfo>,
}

/// Params of `registry.search`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct RegistrySearchParams {
    /// Words to match in ids, names and descriptions; everything when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Revalidate the indexes now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<bool>,
}

/// A core plugin a registry offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct RegistryResult {
    /// Plugin id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The manifest's `kind`.
    pub kind: Vec<String>,
    /// The publisher's display name.
    pub publisher: String,
    /// The publisher's key id.
    pub key: String,
    /// The registry's index URL.
    pub registry: String,
    /// The registry's name.
    pub registry_name: String,
    /// Source repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// The latest version this core can run.
    pub version: String,
    /// What that version asks for.
    pub requested: Permissions,
    /// Whether it is in the registry's starter pack.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub starter: bool,
}

/// Result of `registry.search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct RegistrySearchResult {
    /// Matches, ordered by registry then id.
    pub results: Vec<RegistryResult>,
    /// When the oldest index used was fetched (RFC 3339); absent when no registry could
    /// be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched: Option<String>,
}

/// A result naming one plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginResult {
    /// The plugin.
    pub plugin: InstalledPlugin,
}

/// Params naming one plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginRef {
    /// Plugin id.
    pub plugin: String,
}

/// Params of `plugin.enable`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginEnableParams {
    /// Plugin id.
    pub plugin: String,
    /// The permissions granted; must equal the plugin's `requested`.
    pub grant: Permissions,
}

/// Params of `plugin.uninstall`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PluginUninstallParams {
    /// Plugin id.
    pub plugin: String,
    /// Also delete its kv store and data directory (default false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_data: Option<bool>,
}

/// A provider a core offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ProviderInfo {
    /// Qualified id: `<plugin id>/<local id>`.
    pub id: String,
    /// Display name, from the manifest.
    pub name: String,
    /// The plugin whose manifest declares it.
    pub plugin: String,
    /// The provider's icon from its manifest, as a `data:` URL of an SVG or PNG image
    /// (since 0.11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Result of `provider.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProviderListResult {
    /// Available providers, by id.
    pub providers: Vec<ProviderInfo>,
}

/// Params of `provider.models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelsParams {
    /// Qualified provider id.
    pub provider: String,
}

/// A model a provider offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    /// Model id, as passed to `thread.create`.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Capability strings (`docs/spec/05-providers.md#capabilities`).
    pub capabilities: Vec<String>,
    /// Settings the model offers, such as effort or speed (since 0.11).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<ModelOption>>", optional)]
    pub options: Vec<ModelOption>,
}

/// A setting a model offers, such as effort or speed (since 0.11, ADR 0033).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ModelOption {
    /// Id, the key in `Thread.options`.
    pub id: String,
    /// Display label.
    pub label: String,
    /// What kind of setting it is: `effort` and `speed` are documented; clients show
    /// other kinds generically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The values it can take, in display order.
    pub values: Vec<ModelOptionValue>,
    /// The value used when none is chosen; absent means the agent's own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// One value a [`ModelOption`] can take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ModelOptionValue {
    /// The value.
    pub value: String,
    /// Display label.
    pub label: String,
    /// Optional longer description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Result of `provider.models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelsResult {
    /// Models; the first is the default.
    pub models: Vec<ModelInfo>,
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

/// Params of `workspace.files.list` (since 0.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct WorkspaceFilesListParams {
    /// Workspace to search.
    pub workspace: WorkspaceId,
    /// Optional linked worktree directory; defaults to the workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Case-insensitive substring filter on relative file paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Maximum number of paths to return (default 50, capped at 500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Result of `workspace.files.list` (since 0.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilesListResult {
    /// Relative paths using forward slashes.
    pub files: Vec<String>,
    /// True when more files matched than `limit`.
    #[serde(default)]
    pub truncated: bool,
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
    /// Include side threads (default false; since 0.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_side: Option<bool>,
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
    /// Permission mode for the thread; defaults to `standard`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// Optional linked Git worktree path for this thread (keeps the thread under `workspace`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    /// Model settings, by [`ModelOption`] id, checked against the model (since 0.11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<std::collections::BTreeMap<String, String>>,
    /// Parent thread id for a linked side thread (since 0.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ThreadId>,
    /// True when creating a side thread hidden from the main sidebar list (since 0.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<bool>,
}

/// Params of `thread.fork` (since 0.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ThreadForkParams {
    /// Source thread to fork.
    pub thread: ThreadId,
    /// Copy events up to and including this turn (mutually exclusive with `beforeTurn`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_turn: Option<TurnId>,
    /// Copy events strictly before this turn (mutually exclusive with `atTurn`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_turn: Option<TurnId>,
    /// Optional title for the new thread.
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

/// Params of `thread.update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ThreadUpdateParams {
    /// The thread.
    pub thread: ThreadId,
    /// New provider id (ADR 0029, since 0.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// New model id (validated against the thread's provider, or the new `provider`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Handoff mode when changing `provider` (`transcript` by default; since 0.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<crate::model::HandoffMode>,
    /// New permission mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// New model settings; replaces all of them, and an empty map clears them. Values
    /// the (new) model does not offer are refused (since 0.11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<std::collections::BTreeMap<String, String>>,
    /// New title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Result of `thread.update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadUpdateResult {
    /// The updated thread.
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
    /// Messages waiting in the queue, in order (since 0.7).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<QueueEntry>>", optional)]
    pub queue: Vec<QueueEntry>,
    /// Why the queue is held, if it is (since 0.7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub queue_hold: Option<QueueHold>,
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

/// Result of `thread.send` and `thread.queue.sendNow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSendResult {
    /// The turn this message will run as.
    pub turn: TurnId,
    /// True if another turn is running and this one waits in the queue.
    pub queued: bool,
    /// The queue entry, when the message was queued (since 0.7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub entry: Option<QueueEntryId>,
}

/// What happened to a message sent with `thread.steer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SteerStatus {
    /// The running turn's provider will get it; `input.steered` follows when it takes it.
    Accepted,
    /// The turn cannot be steered (the model lacks `steer`, or the message has
    /// attachments): it went to the front of the queue instead.
    Queued,
}

/// Result of `thread.steer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSteerResult {
    /// What happened.
    pub status: SteerStatus,
    /// The running turn the message is for.
    pub turn: TurnId,
    /// The queue entry, for `queued`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub entry: Option<QueueEntryId>,
}

/// Params of `thread.defer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDeferParams {
    /// The thread.
    pub thread: ThreadId,
    /// The message to send when the trigger fires.
    pub input: UserInput,
    /// Condition to wait for.
    pub trigger: QueueTrigger,
}

/// Params naming a queued message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntryRef {
    /// The thread.
    pub thread: ThreadId,
    /// The entry.
    pub entry: QueueEntryId,
}

/// Params of `thread.queue.update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueUpdateParams {
    /// The thread.
    pub thread: ThreadId,
    /// The entry.
    pub entry: QueueEntryId,
    /// The new message.
    pub input: UserInput,
}

/// Result of `thread.queue.update`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntryResult {
    /// The entry after the change.
    pub entry: QueueEntry,
}

/// Params of `thread.queue.move`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueMoveParams {
    /// The thread.
    pub thread: ThreadId,
    /// The entry to move.
    pub entry: QueueEntryId,
    /// The entry it goes before; absent moves it to the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub before: Option<QueueEntryId>,
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

/// Params of `secret.set`. The value is never returned, logged or echoed.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SecretSetParams {
    /// Secret name (`[a-z0-9-]`, as in `permissions.secrets`).
    pub name: String,
    /// The value.
    pub value: String,
}

impl std::fmt::Debug for SecretSetParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretSetParams")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Params naming one secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    /// Secret name.
    pub name: String,
}

/// One secret name as `secret.list` shows it: never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SecretInfo {
    /// Secret name.
    pub name: String,
    /// Whether a value is stored.
    pub set: bool,
    /// Installed plugins whose `permissions.secrets` lists it.
    pub plugins: Vec<String>,
}

/// Result of `secret.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SecretListResult {
    /// Every name a plugin declares or that is set, sorted.
    pub secrets: Vec<SecretInfo>,
}

/// Result of `settings.schema`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSchemaResult {
    /// The plugin's settings JSON Schema.
    #[ts(type = "Record<string, unknown>")]
    pub schema: serde_json::Value,
}

/// Result of `settings.get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsGetResult {
    /// The stored values, with the schema's defaults for the rest; no secret fields.
    #[ts(type = "Record<string, unknown>")]
    pub values: serde_json::Value,
}

/// Params of `settings.set`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSetParams {
    /// Plugin id.
    pub plugin: String,
    /// Every value (they replace the stored ones); secret fields are refused.
    #[ts(type = "Record<string, unknown>")]
    pub values: serde_json::Value,
}

/// Result of `plugin.workspaces`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginWorkspacesResult {
    /// Linked workspace ids.
    pub workspaces: Vec<WorkspaceId>,
}

/// Params of `plugin.setWorkspaces`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PluginSetWorkspacesParams {
    /// Plugin id.
    pub plugin: String,
    /// The workspaces it may act in when its `permissions.workspaces` is `linked`.
    pub workspaces: Vec<WorkspaceId>,
}

/// A schedule as `schedule.list` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ScheduleInfo {
    /// The owning plugin.
    pub plugin: String,
    /// Its key.
    pub key: String,
    /// What it does, in the plugin's words.
    pub description: String,
    /// The cron expression.
    pub cron: String,
    /// IANA time zone; absent for the core's local zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    /// False while paused.
    pub enabled: bool,
    /// `skip` or `once`.
    pub missed: String,
    /// `manifest` or `plugin`.
    pub source: String,
    /// Next run, RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run: Option<String>,
    /// Last run, RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<String>,
    /// `ran`, `failed`, `missed` or `skipped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<String>,
}

/// Result of `schedule.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleListResult {
    /// Every schedule, by plugin and key.
    pub schedules: Vec<ScheduleInfo>,
}

/// Params naming one schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRef {
    /// The plugin.
    pub plugin: String,
    /// The schedule's key.
    pub key: String,
}

/// Params of `schedule.setEnabled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSetEnabledParams {
    /// The plugin.
    pub plugin: String,
    /// The schedule's key.
    pub key: String,
    /// Run (true) or pause (false).
    pub enabled: bool,
}

/// Params of `gateway.attach`. The token is never logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GatewayAttachParams {
    /// The session's token (`KUMIKIT_MCP_TOKEN`).
    pub token: String,
}

impl std::fmt::Debug for GatewayAttachParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayAttachParams")
            .finish_non_exhaustive()
    }
}

/// What a core plugin's command or method acts for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The core.
    #[default]
    Core,
    /// The workspace it runs in.
    Workspace,
}

/// A command a core plugin contributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CommandInfo {
    /// The plugin.
    pub plugin: String,
    /// Command id.
    pub id: String,
    /// Title.
    pub title: String,
    /// What it acts for.
    pub scope: Scope,
    /// A UI condition for showing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub when: Option<String>,
}

/// Result of `command.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CommandListResult {
    /// Commands of enabled plugins.
    pub commands: Vec<CommandInfo>,
}

/// Params of `command.run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CommandRunParams {
    /// The plugin.
    pub plugin: String,
    /// Command id.
    pub command: String,
    /// Arguments (a JSON object), for example a submitted form's values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "Record<string, unknown>")]
    pub args: Option<serde_json::Value>,
    /// The workspace, for `workspace` commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace: Option<WorkspaceId>,
}

/// What running a command shows (`command.run`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    /// A line to show the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message: Option<String>,
    /// A declarative view to show (`docs/spec/07-ui-plugins.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "Record<string, unknown>")]
    pub view: Option<serde_json::Value>,
}

/// Core-level configuration returned by `core.config.get` and `core.config.set`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CoreConfigResult {
    /// Active Kumikit home directory where Kumikit creates folders such as `worktrees/`.
    pub home_dir: String,
    /// Platform default Kumikit home directory (for example `~/.kumikit`).
    pub default_home_dir: String,
}

/// Params of `core.config.set`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct CoreConfigSetParams {
    /// Custom Kumikit home directory; omit or set to an empty string to reset to the platform default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_dir: Option<String>,
}

/// One managed Git worktree stored under `<homeDir>/worktrees/` (`core.worktrees.list`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct WorktreeEntry {
    /// Canonical absolute path of the worktree directory.
    pub path: String,
    /// Repository directory name under `<homeDir>/worktrees/<repo>/`.
    pub repo: String,
    /// Worktree directory name (`<branch-slug>`).
    pub name: String,
    /// Active Git branch (or short commit SHA) checked out in this worktree, if readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Total disk space occupied by files inside this worktree, in bytes.
    pub size_bytes: u64,
    /// Number of threads currently associated with this worktree on this core.
    pub thread_count: u32,
    /// True when any linked thread currently has a running turn.
    #[serde(default)]
    pub in_use: bool,
}

/// Result of `core.worktrees.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeListResult {
    /// Active `<homeDir>/worktrees` directory that was scanned.
    pub root: String,
    /// Worktrees found under `root`, largest first.
    pub worktrees: Vec<WorktreeEntry>,
    /// Sum of `size_bytes` across all listed worktrees.
    pub total_bytes: u64,
}

/// Params of `core.worktrees.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeDeleteParams {
    /// Absolute path of the managed worktree directory inside `<homeDir>/worktrees/` to delete.
    pub path: String,
}

/// One local Git branch reported by `workspace.git.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct GitBranch {
    /// Short branch name, for example `main` or `m6-ui-design`.
    pub name: String,
    /// True if this branch is currently checked out in the queried workspace.
    pub current: bool,
    /// Absolute path of the worktree that has this branch checked out, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

/// Git repository and branch state of a workspace (`workspace.git.status` / `workspace.git.checkout`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct WorkspaceGitInfo {
    /// True when the workspace directory is inside a Git repository.
    pub is_repo: bool,
    /// Active branch name, or short commit SHA when `detached` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// True when HEAD is detached.
    #[serde(default)]
    pub detached: bool,
    /// True when this checkout is a linked Git worktree rather than the primary working tree.
    #[serde(default)]
    pub is_worktree: bool,
    /// Canonical root of the repository (or linked worktree) checkout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    /// Repository's default branch (`main`, `master`, or `origin/HEAD`), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// Local branches in the repository.
    #[serde(default)]
    pub branches: Vec<GitBranch>,
}

/// Params of `workspace.git.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct WorkspaceGitStatusParams {
    /// Workspace to inspect.
    pub workspace: WorkspaceId,
    /// Optional linked worktree path belonging to this repository; defaults to the workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Params of `workspace.git.checkout`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct WorkspaceGitCheckoutParams {
    /// Workspace to switch.
    pub workspace: WorkspaceId,
    /// Existing local branch name to check out.
    pub branch: String,
    /// Optional linked worktree path belonging to this repository; defaults to the workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Params of `workspace.git.worktreeCreate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct WorkspaceWorktreeCreateParams {
    /// Source Git workspace.
    pub workspace: WorkspaceId,
    /// Branch name for the new worktree (created if it does not already exist).
    pub branch: String,
    /// Base branch or ref to branch from when creating a new branch; defaults to HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
}

/// Result of `workspace.git.worktreeCreate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceWorktreeCreateResult {
    /// Canonical absolute path of the newly created linked worktree directory.
    pub path: String,
    /// Git state of the new worktree checkout.
    pub git: WorkspaceGitInfo,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_values_stay_out_of_debug_output() {
        let p = SecretSetParams {
            name: "bot-token".into(),
            value: "123:abc".into(),
        };
        assert!(!format!("{p:?}").contains("123:abc"));
    }

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
