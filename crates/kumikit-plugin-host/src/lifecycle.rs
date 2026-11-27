//! The plugin host: turns enabled plugins into kernel providers and manages their
//! instances (`docs/spec/04-plugin-host.md#lifecycle`, `docs/spec/05-providers.md`).
//!
//! - Providers are registered with the kernel from manifests alone; nothing is
//!   instantiated until one is used. Their ids are qualified, `<plugin>/<local id>`.
//! - A delegated entry (`acp = { … }`) is served by the enabled plugin that declares the
//!   backend key, after its table passes the backend's schema; its sessions run the
//!   backend's code with the contributing plugin's grants.
//! - A provider plugin has one control instance (for `list-models`) and one instance per
//!   session. Instances idle for [`HostConfig::idle_after`] are dropped.
//! - A trap or limit breach discards every instance of the plugin and puts it in
//!   backoff; the next use after the backoff starts it again.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use kumikit_kernel::Kernel;
use kumikit_kernel::grants::Grants;
use kumikit_kernel::plugins::PluginRecord;
use kumikit_kernel::provider::{
    BoxFuture, EventSink, Model, Provider, ProviderError, ProviderResult, SessionConfig,
};
use kumikit_protocol::model::{ApprovalId, Decision, ThreadId, UserInput};

use crate::manager::{HostError, Manager, PluginView};
use crate::manifest::{Manifest, check_backend_entry};
use crate::runtime::bindings::provider::kumikit::provider::types as wit;
use crate::runtime::{
    CallContext, CallError, CommandResult, Identity, Instance, InstanceConfig, SessionShared,
    WorkspaceRef,
};

/// Lifecycle settings.
#[derive(Clone)]
pub struct HostConfig {
    /// Drop an instance after this long without calls and with no running turn.
    pub idle_after: Duration,
    /// First backoff after a failure; doubles with each failure in a row.
    pub backoff_base: Duration,
    /// Longest backoff.
    pub backoff_cap: Duration,
    /// A plugin that has not failed for this long starts again from `backoff_base`.
    pub backoff_reset: Duration,
    /// After a cancel, how long a session's `send` may keep running before its instance
    /// is discarded and its processes killed.
    pub cancel_grace: Duration,
    /// Time given to `deactivate` before an instance is dropped.
    pub deactivate_limit: Duration,
    /// How long a provider's `list-models` result is cached before probing again.
    pub models_ttl: Duration,
    /// Maximum time a delegated `list-models` probe may run before it is cancelled.
    pub models_probe_timeout: Duration,
    /// Monotonic time; tests replace it.
    pub now: Arc<dyn Fn() -> Instant + Send + Sync>,
}

impl Default for HostConfig {
    fn default() -> Self {
        HostConfig {
            idle_after: Duration::from_secs(600),
            backoff_base: Duration::from_secs(1),
            backoff_cap: Duration::from_secs(300),
            backoff_reset: Duration::from_secs(600),
            cancel_grace: Duration::from_secs(2),
            deactivate_limit: Duration::from_secs(2),
            models_ttl: Duration::from_secs(300),
            models_probe_timeout: Duration::from_secs(10),
            now: Arc::new(Instant::now),
        }
    }
}

impl std::fmt::Debug for HostConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostConfig")
            .field("idle_after", &self.idle_after)
            .field("backoff_base", &self.backoff_base)
            .finish_non_exhaustive()
    }
}

/// An enabled plugin and the providers it makes available, for `core.hello`.
#[derive(Debug, Clone)]
pub struct EnabledPlugin {
    /// The record.
    pub record: PluginRecord,
    /// The manifest.
    pub manifest: Arc<Manifest>,
    /// Qualified ids of its providers that are available now.
    pub providers: Vec<String>,
}

/// A provider as registered: where its calls go.
#[derive(Debug, Clone, PartialEq)]
struct ProviderSpec {
    /// `<contributor>/<local>`.
    id: String,
    local: String,
    /// Display name from the manifest.
    name: String,
    /// The plugin whose manifest declares it (its grants apply to sessions).
    contributor: String,
    /// The plugin whose code serves it.
    backend: String,
    /// The backend table, as JSON, for delegated entries.
    config: Option<String>,
    /// The manifest icon as a `data:` URL (ADR 0033).
    icon: Option<String>,
}

/// Reads a provider's icon from its package into a `data:` URL. A missing, unreadable or
/// oversized file gives no icon (the manifest check refuses those at install).
async fn icon_data_url(package: PathBuf, path: &str) -> Option<String> {
    use base64::Engine as _;
    let media = crate::manifest::icon_media_type(path)?;
    let file = package.join(path);
    let bytes = tokio::task::spawn_blocking(move || {
        let meta = std::fs::metadata(&file).ok()?;
        (meta.len() <= crate::manifest::ICON_MAX_BYTES).then(|| std::fs::read(&file).ok())?
    })
    .await
    .ok()??;
    Some(format!(
        "data:{media};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// A plugin's error as a turn's error text. A denied host call keeps its code in front,
/// so the user (and a client) can tell a refused permission from other failures.
fn guest_message(e: crate::runtime::bindings::base::kumikit::host::types::Error) -> String {
    use crate::runtime::bindings::base::kumikit::host::types::ErrorCode;
    match e.code {
        ErrorCode::PermissionDenied => format!("permission_denied: {}", e.message),
        _ => e.message,
    }
}

/// A guest's error as a host error: the codes clients can act on keep their meaning.
fn guest_error(
    plugin: &str,
    e: crate::runtime::bindings::base::kumikit::host::types::Error,
) -> HostError {
    use crate::runtime::bindings::base::kumikit::host::types::ErrorCode as C;
    match e.code {
        C::PermissionDenied => HostError::Denied(format!("permission_denied: {}", e.message)),
        C::NotFound => HostError::NotFound(e.message),
        C::InvalidArgument | C::Limit => HostError::Invalid(e.message),
        C::Conflict | C::Cancelled => HostError::Conflict(e.message),
        _ => HostError::PluginFailed(format!("{plugin}: {}", e.message)),
    }
}

/// A command a plugin contributes, as `command.list` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInfo {
    /// The plugin.
    pub plugin: String,
    /// The command id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Whether it acts for a workspace.
    pub workspace: bool,
    /// Its UI condition.
    pub when: Option<String>,
}

/// A tool of an enabled plugin, as the gateway lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    /// The plugin.
    pub plugin: String,
    /// Its name in the manifest.
    pub name: String,
    /// Its MCP name (`<plugin id with _>__<name>`).
    pub mcp_name: String,
    /// Title for people.
    pub title: String,
    /// Description for the agent.
    pub description: String,
    /// The JSON Schema of its arguments.
    pub input_schema: serde_json::Value,
    /// Whether it needs an approval.
    pub write: bool,
}

/// What a tool call returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    /// Text for the agent.
    pub text: String,
    /// Structured result, JSON text.
    pub structured: Option<String>,
    /// The tool failed.
    pub is_error: bool,
}

/// The qualified id of a provider.
pub fn qualified(plugin: &str, local: &str) -> String {
    format!("{plugin}/{local}")
}

struct Session {
    instance: Instance,
    shared: Arc<SessionShared>,
    thread: ThreadId,
    /// `0` when idle, otherwise the generation of the active `send` call.
    busy_gen: Arc<AtomicU64>,
    next_gen: AtomicU64,
    /// Serializes turns on this session so a new turn never clears a cancelled turn's
    /// `cancel_pending` flag or overwrites its `EventSink` before the guest winds down.
    turn_lock: Arc<tokio::sync::Mutex<()>>,
    last_used: Mutex<Instant>,
}

#[derive(Default)]
struct PluginRun {
    control: Option<(Instance, Instant)>,
    sessions: HashMap<String, Arc<Session>>,
    failures: u32,
    last_failure: Option<Instant>,
    retry_at: Option<Instant>,
    failure: Option<String>,
}

/// Why a plugin is not running, for `plugin.list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// What happened.
    pub message: String,
    /// Time left before it may start again, if still in backoff.
    pub retry_in: Option<Duration>,
}

type ModelsSlot = Arc<tokio::sync::Mutex<Option<(Instant, Vec<Model>)>>>;

struct Inner {
    manager: Manager,
    config: HostConfig,
    runs: Mutex<HashMap<String, PluginRun>>,
    registered: Mutex<BTreeMap<String, ProviderSpec>>,
    models_cache: Mutex<HashMap<String, ModelsSlot>>,
    reaper: AtomicBool,
    /// Bumped after the available providers were recomputed.
    changed: tokio::sync::watch::Sender<u64>,
}

/// The plugin host. Cheap to clone.
#[derive(Clone)]
pub struct PluginHost {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PluginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginHost").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // Plain maps; they stay consistent if a holder panicked.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn perr(e: impl std::fmt::Display) -> ProviderError {
    ProviderError::new(e.to_string())
}

impl PluginHost {
    /// Registers the enabled plugins' providers and starts plugins that ask for
    /// `startup` activation (in the background). Instantiates nothing else.
    pub async fn open(manager: Manager, config: HostConfig) -> PluginHost {
        let host = PluginHost {
            inner: Arc::new(Inner {
                manager,
                config,
                runs: Mutex::new(HashMap::new()),
                registered: Mutex::new(BTreeMap::new()),
                models_cache: Mutex::new(HashMap::new()),
                reaper: AtomicBool::new(false),
                changed: tokio::sync::watch::Sender::new(0),
            }),
        };
        host.reconcile().await;
        for (record, manifest) in host.inner.manager.enabled() {
            if manifest.activation.on.iter().any(|t| t == "startup") && manifest.wasm().is_some() {
                let h = host.clone();
                tokio::spawn(async move {
                    if let Err(e) = h.control(&record.id).await {
                        tracing::warn!(plugin = %record.id, error = %e, "startup activation failed");
                    }
                });
            }
        }
        host
    }

    /// The plugin manager.
    pub fn manager(&self) -> &Manager {
        &self.inner.manager
    }

    /// A receiver that sees a new value whenever the enabled plugins or the providers
    /// they make available change. Fires after providers are registered, so a listener
    /// that reads [`PluginHost::enabled`] sees the new state.
    pub fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.inner.changed.subscribe()
    }

    fn kernel(&self) -> &Kernel {
        self.inner.manager.kernel()
    }

    fn now(&self) -> Instant {
        (self.inner.config.now)()
    }

    // ---- management, with provider registration ----

    /// Installs a package (see [`Manager::install`]).
    pub async fn install(
        &self,
        package: &std::path::Path,
        source: kumikit_kernel::plugins::PluginSource,
    ) -> Result<PluginView, HostError> {
        self.inner.manager.install(package, source).await
    }

    /// Installs a package checked against a registry's entry (see
    /// [`Manager::install_checked`]).
    pub async fn install_checked(
        &self,
        package: &std::path::Path,
        source: kumikit_kernel::plugins::PluginSource,
        expected: Option<&crate::manager::Expected>,
    ) -> Result<PluginView, HostError> {
        self.inner
            .manager
            .install_checked(package, source, expected)
            .await
    }

    /// Whether a turn runs on one of the plugin's providers, or on one its backend serves.
    pub fn has_running_turns(&self, id: &str) -> bool {
        let providers: Vec<String> = lock(&self.inner.registered)
            .values()
            .filter(|s| s.contributor == id || s.backend == id)
            .map(|s| s.id.clone())
            .collect();
        self.inner.manager.kernel().turns_running_on(&providers)
    }

    /// Updates a plugin to the version in `package` (10-distribution, Updates): refused
    /// while one of its turns runs; an enabled plugin is disabled around the swap and
    /// enabled again with `grant`. If the swap fails, the old version stays as it was.
    pub async fn update(
        &self,
        id: &str,
        package: &std::path::Path,
        source: kumikit_kernel::plugins::PluginSource,
        expected: Option<&crate::manager::Expected>,
        grant: Option<Grants>,
    ) -> Result<PluginView, HostError> {
        if self.has_running_turns(id) {
            return Err(HostError::Conflict(format!(
                "a turn of {id} is running; wait for it to end or cancel it, then update"
            )));
        }
        let before = self.inner.manager.get(id)?;
        let was_enabled = before.record.state == kumikit_kernel::plugins::PluginState::Enabled;
        if was_enabled {
            self.inner.manager.disable(id).await?;
            self.reconcile().await;
            self.unload(id, "updating").await;
        }
        let replaced = self
            .inner
            .manager
            .replace(id, package, source, expected, grant.clone())
            .await;
        let view = match replaced {
            Ok(v) => v,
            Err(e) => {
                if was_enabled && let Some(old) = &before.record.granted {
                    let _ = self.inner.manager.enable(id, old).await;
                    self.reconcile().await;
                }
                self.inner.changed.send_modify(|v| *v += 1);
                return Err(e);
            }
        };
        let view = match (was_enabled, &grant) {
            (true, Some(g)) => {
                let enabled = self.inner.manager.enable(id, g).await;
                self.reconcile().await;
                enabled?
            }
            _ => view,
        };
        self.inner.changed.send_modify(|v| *v += 1);
        Ok(view)
    }

    /// Grants and enables a plugin, then registers its providers.
    pub async fn enable(&self, id: &str, grant: &Grants) -> Result<PluginView, HostError> {
        let view = self.inner.manager.enable(id, grant).await?;
        self.reconcile().await;
        self.inner.changed.send_modify(|v| *v += 1);
        Ok(view)
    }

    /// Disables a plugin: its providers go away (running turns are cancelled) and its
    /// instances and processes are stopped.
    pub async fn disable(&self, id: &str) -> Result<PluginView, HostError> {
        let view = self.inner.manager.disable(id).await?;
        self.reconcile().await;
        self.inner.changed.send_modify(|v| *v += 1);
        self.unload(id, "disabled").await;
        Ok(view)
    }

    /// Removes a plugin (see [`Manager::uninstall`]) after stopping it.
    pub async fn uninstall(&self, id: &str, delete_data: bool) -> Result<(), HostError> {
        let was_enabled = self.inner.manager.get(id)?.record.state
            == kumikit_kernel::plugins::PluginState::Enabled;
        if was_enabled {
            self.inner.manager.disable(id).await?;
            self.reconcile().await;
        }
        self.unload(id, "removed").await;
        lock(&self.inner.runs).remove(id);
        self.inner.manager.uninstall(id, delete_data).await?;
        if was_enabled {
            self.inner.changed.send_modify(|v| *v += 1);
        }
        Ok(())
    }

    /// Enabled plugins with the providers available now.
    pub fn enabled(&self) -> Vec<EnabledPlugin> {
        let registered = lock(&self.inner.registered).clone();
        self.inner
            .manager
            .enabled()
            .into_iter()
            .map(|(record, manifest)| {
                let providers = registered
                    .values()
                    .filter(|s| s.contributor == record.id)
                    .map(|s| s.id.clone())
                    .collect();
                EnabledPlugin {
                    record,
                    manifest,
                    providers,
                }
            })
            .collect()
    }

    /// The plugin's last failure, while it is in backoff or until it runs again.
    pub fn failure(&self, id: &str) -> Option<Failure> {
        let runs = lock(&self.inner.runs);
        let run = runs.get(id)?;
        let message = run.failure.clone()?;
        let now = self.now();
        Some(Failure {
            message,
            retry_in: run.retry_at.and_then(|t| t.checked_duration_since(now)),
        })
    }

    /// Starts the kernel's scheduler with this host as its runner (`kumikit serve` only:
    /// CLI commands that open the data directory must not run schedules).
    pub fn start_scheduler(&self) {
        let runner = ScheduleRunner {
            host: Arc::downgrade(&self.inner),
        };
        self.inner
            .manager
            .kernel()
            .start_scheduler(Arc::new(runner));
    }

    /// Makes enabled plugins' `schedule:<cron>` triggers their manifest schedules, and
    /// wakes the scheduler (a plugin may have been enabled or disabled).
    async fn sync_schedules(&self, enabled: &[(PluginRecord, Arc<Manifest>)]) {
        let kernel = self.inner.manager.kernel();
        let now = jiff::Timestamp::now();
        for (record, manifest) in enabled {
            let crons: Vec<String> = manifest
                .activation
                .on
                .iter()
                .filter_map(|t| t.strip_prefix("schedule:").map(|c| c.trim().to_owned()))
                .collect();
            if let Err(e) = kernel.schedule_sync_manifest(&record.id, &crons, now).await {
                tracing::warn!(plugin = %record.id, error = %e, "cannot record the manifest's schedules");
            }
        }
        kernel.schedules_changed();
    }

    /// The tools of every enabled plugin, with their schemas (read from the packages).
    pub fn tools(&self) -> Vec<ToolInfo> {
        let mut out = Vec::new();
        for (record, manifest) in self.inner.manager.enabled() {
            for t in &manifest.contributes.tools {
                let path = self
                    .inner
                    .manager
                    .dirs()
                    .package(&record.id, &record.version)
                    .join(&t.input);
                let schema = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok());
                let Some(input_schema) = schema else {
                    tracing::warn!(plugin = %record.id, tool = %t.name, "cannot read the tool's input schema; not listed");
                    continue;
                };
                out.push(ToolInfo {
                    plugin: record.id.clone(),
                    name: t.name.clone(),
                    mcp_name: crate::manifest::mcp_tool_name(&record.id, &t.name),
                    title: t.title.clone(),
                    description: t.description.clone(),
                    input_schema,
                    write: t.effect == crate::manifest::Effect::Write,
                });
            }
        }
        out
    }

    /// Runs a tool in its plugin's control instance, acting for `workspace_root`.
    pub async fn call_tool(
        &self,
        plugin: &str,
        name: &str,
        input: &serde_json::Value,
        thread: &str,
        workspace_root: &str,
    ) -> Result<ToolOutcome, HostError> {
        use crate::runtime::bindings::tool::exports::kumikit::tool::tool::ToolCall;
        self.enabled_manifest(plugin)?;
        let control = self
            .control(plugin)
            .await
            .map_err(|e| HostError::PluginFailed(e.to_string()))?;
        let r = control
            .call_tool(ToolCall {
                name: name.to_owned(),
                input: input.to_string(),
                thread: thread.to_owned(),
                workspace_root: workspace_root.to_owned(),
            })
            .await;
        let r = self.guest_result(plugin, r).await?;
        Ok(ToolOutcome {
            text: r.text,
            structured: r.structured,
            is_error: r.is_error,
        })
    }

    /// The commands of every enabled plugin, by plugin then manifest order.
    pub fn commands(&self) -> Vec<CommandInfo> {
        self.inner
            .manager
            .enabled()
            .into_iter()
            .flat_map(|(record, manifest)| {
                manifest
                    .contributes
                    .commands
                    .iter()
                    .map(|c| CommandInfo {
                        plugin: record.id.clone(),
                        id: c.id.clone(),
                        title: c.title.clone(),
                        workspace: c.scope == crate::manifest::Scope::Workspace,
                        when: c.when.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// An enabled plugin's manifest, for routing calls to it.
    fn enabled_manifest(&self, plugin: &str) -> Result<Arc<Manifest>, HostError> {
        self.inner
            .manager
            .enabled()
            .into_iter()
            .find(|(r, _)| r.id == plugin)
            .map(|(_, m)| m)
            .ok_or_else(|| HostError::NotFound(format!("plugin {plugin} is not enabled")))
    }

    /// Whether `method` is a `workspace`-scoped method of the enabled `plugin`; `None` if
    /// the plugin does not declare it (or is not enabled).
    pub fn method_scope(&self, plugin: &str, method: &str) -> Option<bool> {
        let m = self.enabled_manifest(plugin).ok()?;
        m.contributes
            .methods
            .iter()
            .find(|x| x.name == method)
            .map(|x| x.scope == crate::manifest::Scope::Workspace)
    }

    /// Whether `command` is a `workspace`-scoped command of the enabled `plugin`.
    pub fn command_scope(&self, plugin: &str, command: &str) -> Result<bool, HostError> {
        let m = self.enabled_manifest(plugin)?;
        m.contributes
            .commands
            .iter()
            .find(|x| x.id == command)
            .map(|x| x.scope == crate::manifest::Scope::Workspace)
            .ok_or_else(|| HostError::NotFound(format!("{plugin} has no command {command}")))
    }

    /// Calls a plugin method in its control instance (`<plugin>/<method>`), acting for
    /// `workspace`. `params` and the result are JSON.
    pub async fn call_method(
        &self,
        plugin: &str,
        method: &str,
        params: &serde_json::Value,
        workspace: Option<WorkspaceRef>,
    ) -> Result<serde_json::Value, HostError> {
        self.enabled_manifest(plugin)?;
        let control = self
            .control(plugin)
            .await
            .map_err(|e| HostError::PluginFailed(e.to_string()))?;
        let ctx = CallContext { workspace };
        let r = control
            .handle_rpc(method.to_owned(), params.to_string(), ctx)
            .await;
        let text = self.guest_result(plugin, r).await?;
        serde_json::from_str(&text).map_err(|e| {
            HostError::PluginFailed(format!(
                "{plugin}/{method} answered something that is not JSON: {e}"
            ))
        })
    }

    /// Runs a plugin command in its control instance, acting for `workspace`. Returns the
    /// message and the view (JSON) it shows.
    pub async fn run_command(
        &self,
        plugin: &str,
        command: &str,
        args: &serde_json::Value,
        workspace: Option<WorkspaceRef>,
    ) -> Result<(Option<String>, Option<serde_json::Value>), HostError> {
        self.command_scope(plugin, command)?;
        let control = self
            .control(plugin)
            .await
            .map_err(|e| HostError::PluginFailed(e.to_string()))?;
        let ctx = CallContext { workspace };
        let r = control
            .run_command(command.to_owned(), args.to_string(), ctx)
            .await;
        let CommandResult { message, view } = self.guest_result(plugin, r).await?;
        let view = view
            .map(|v| {
                serde_json::from_str(&v).map_err(|e| {
                    HostError::PluginFailed(format!(
                        "{plugin}: command {command} gave a view that is not JSON: {e}"
                    ))
                })
            })
            .transpose()?;
        Ok((message, view))
    }

    /// Maps an export call's result; a trap fails the plugin.
    async fn guest_result<T>(
        &self,
        plugin: &str,
        r: Result<
            Result<T, crate::runtime::bindings::base::kumikit::host::types::Error>,
            CallError,
        >,
    ) -> Result<T, HostError> {
        match r {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(guest_error(plugin, e)),
            Err(e) => {
                if matches!(e, CallError::Trap { .. }) {
                    self.failed(plugin, &e.to_string()).await;
                }
                Err(HostError::PluginFailed(e.to_string()))
            }
        }
    }

    /// Instances running for a plugin: (control instance, sessions).
    pub fn instances(&self, id: &str) -> (bool, usize) {
        lock(&self.inner.runs)
            .get(id)
            .map_or((false, 0), |r| (r.control.is_some(), r.sessions.len()))
    }

    /// Computes the providers the enabled plugins make available and registers or
    /// unregisters the difference with the kernel.
    async fn reconcile(&self) {
        let enabled = self.inner.manager.enabled();
        self.sync_schedules(&enabled).await;
        // key -> (backend plugin, its version, its schema file)
        let mut backends: HashMap<String, (String, String, Option<PathBuf>)> = HashMap::new();
        for (record, manifest) in &enabled {
            for b in &manifest.contributes.provider_backends {
                let schema = b.schema.as_ref().map(|s| {
                    self.inner
                        .manager
                        .dirs()
                        .package(&record.id, &record.version)
                        .join(s)
                });
                backends.insert(
                    b.key.clone(),
                    (record.id.clone(), record.version.clone(), schema),
                );
            }
        }
        let mut wanted = BTreeMap::new();
        for (record, manifest) in &enabled {
            for entry in &manifest.contributes.providers {
                let id = qualified(&record.id, &entry.id);
                let icon = match &entry.icon {
                    Some(path) => {
                        let package = self
                            .inner
                            .manager
                            .dirs()
                            .package(&record.id, &record.version);
                        icon_data_url(package, path).await
                    }
                    None => None,
                };
                let spec = match entry.backend() {
                    None => ProviderSpec {
                        id: id.clone(),
                        local: entry.id.clone(),
                        name: entry.name.clone(),
                        contributor: record.id.clone(),
                        backend: record.id.clone(),
                        config: None,
                        icon,
                    },
                    Some((key, table)) => {
                        let Some((backend, backend_version, schema)) = backends.get(key) else {
                            tracing::debug!(provider = %id, key, "no enabled plugin serves this backend");
                            continue;
                        };
                        // A table that passed this backend version's schema once is not
                        // checked again (that would start the schema validator at every
                        // startup); the marker lives with the contributor's files.
                        let marker = self
                            .inner
                            .manager
                            .dirs()
                            .package(&record.id, &record.version)
                            .join(format!(".checked-{backend}-{backend_version}-{}", entry.id));
                        let checked = {
                            let m = marker.clone();
                            tokio::task::spawn_blocking(move || m.is_file())
                                .await
                                .unwrap_or(false)
                        };
                        if let Some(schema) = schema.as_ref().filter(|_| !checked) {
                            let path = schema.clone();
                            let read =
                                tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
                                    .await
                                    .map_err(std::io::Error::other)
                                    .and_then(|r| r);
                            let text = match read {
                                Ok(t) => t,
                                Err(e) => {
                                    tracing::warn!(provider = %id, error = %e, "cannot read the backend schema");
                                    continue;
                                }
                            };
                            if let Err(e) = check_backend_entry(&text, table) {
                                tracing::warn!(provider = %id, error = %e, "provider entry does not match its backend's schema");
                                continue;
                            }
                            let m = marker.clone();
                            let written =
                                tokio::task::spawn_blocking(move || std::fs::write(m, b""))
                                    .await
                                    .map_err(std::io::Error::other)
                                    .and_then(|r| r);
                            if let Err(e) = written {
                                tracing::debug!(provider = %id, error = %e, "cannot record the schema check");
                            }
                        }
                        ProviderSpec {
                            id: id.clone(),
                            local: entry.id.clone(),
                            name: entry.name.clone(),
                            contributor: record.id.clone(),
                            backend: backend.clone(),
                            config: Some(table.to_string()),
                            icon,
                        }
                    }
                };
                wanted.insert(id, spec);
            }
        }

        let current = lock(&self.inner.registered).clone();
        for (id, spec) in &current {
            if wanted.get(id) != Some(spec) {
                self.kernel().unregister_provider(id);
                lock(&self.inner.registered).remove(id);
                lock(&self.inner.models_cache).remove(id);
            }
        }
        for (id, spec) in wanted {
            if current.get(&id) == Some(&spec) {
                continue;
            }
            lock(&self.inner.models_cache).remove(&id);
            let provider = WasmProvider {
                spec: spec.clone(),
                host: Arc::downgrade(&self.inner),
            };
            match self.kernel().register_provider(Arc::new(provider)) {
                Ok(()) => {
                    lock(&self.inner.registered).insert(id, spec);
                }
                Err(e) => tracing::warn!(provider = %id, error = %e, "cannot register provider"),
            }
        }
    }

    // ---- instances ----

    fn models_slot(&self, provider_id: &str) -> ModelsSlot {
        lock(&self.inner.models_cache)
            .entry(provider_id.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(None)))
            .clone()
    }

    fn clear_models_cache_for_plugin(&self, plugin: &str) {
        let affected: Vec<String> = lock(&self.inner.registered)
            .iter()
            .filter(|(_, spec)| spec.backend == plugin || spec.contributor == plugin)
            .map(|(id, _)| id.clone())
            .collect();
        let mut cache = lock(&self.inner.models_cache);
        for id in affected {
            cache.remove(&id);
        }
    }

    /// Creates and returns an isolated empty directory for `list-models` probes of
    /// `plugin` (`<plugin_data>/probe`).
    async fn probe_dir(&self, plugin: &str) -> Result<PathBuf, ProviderError> {
        let dir = self.inner.manager.dirs().plugin_data(plugin).join("probe");
        let to_create = dir.clone();
        tokio::task::spawn_blocking(move || std::fs::create_dir_all(&to_create))
            .await
            .map_err(perr)?
            .map_err(|e| ProviderError::new(format!("cannot create probe directory: {e}")))?;
        Ok(dir)
    }

    fn instance_config(
        &self,
        backend: &str,
        grants_from: &str,
        workspaces: Vec<PathBuf>,
        session: Arc<SessionShared>,
        probe_dir: Option<PathBuf>,
    ) -> Result<InstanceConfig, ProviderError> {
        let code = self.inner.manager.get(backend).map_err(perr)?;
        let manifest = code
            .manifest
            .ok_or_else(|| ProviderError::new(format!("plugin {backend} cannot be loaded")))?;
        let grants = self
            .inner
            .manager
            .get(grants_from)
            .map_err(perr)?
            .record
            .granted
            .unwrap_or_default();
        Ok(InstanceConfig {
            identity: Identity {
                plugin: backend.to_owned(),
                grants_from: grants_from.to_owned(),
                grants,
                workspaces,
            },
            data_dir: self.inner.manager.dirs().plugin_data(backend),
            memory: manifest.memory_limit(),
            call_timeout: manifest.call_timeout(),
            session,
            settings_schema: manifest.contributes.settings.as_ref().map(|f| {
                self.inner
                    .manager
                    .dirs()
                    .package(backend, &code.record.version)
                    .join(f)
            }),
            probe_dir,
        })
    }

    /// Fails fast while the plugin is in backoff.
    fn check_backoff(&self, plugin: &str) -> Result<(), ProviderError> {
        let now = self.now();
        let runs = lock(&self.inner.runs);
        if let Some(at) = runs.get(plugin).and_then(|r| r.retry_at)
            && at > now
        {
            let secs = at.duration_since(now).as_secs_f64().ceil();
            return Err(ProviderError::new(format!(
                "plugin {plugin} failed and is retrying in {secs} s"
            )));
        }
        Ok(())
    }

    async fn start_instance(&self, config: InstanceConfig) -> Result<Instance, ProviderError> {
        let plugin = config.identity.plugin.clone();
        self.check_backoff(&plugin)?;
        let prepared = self.inner.manager.prepared(&plugin).await.map_err(perr)?;
        let started = Instance::start(
            self.inner.manager.runtime(),
            &prepared,
            config,
            self.kernel().clone(),
        )
        .await;
        let instance = match started {
            Ok(i) => i,
            Err(e) => {
                self.failed(&plugin, &e.to_string()).await;
                return Err(perr(e));
            }
        };
        match instance.activate().await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let message = format!("activate failed: {}", e.message);
                self.failed(&plugin, &message).await;
                return Err(ProviderError::new(message));
            }
            Err(e) => {
                self.failed(&plugin, &e.to_string()).await;
                return Err(perr(e));
            }
        }
        self.ensure_reaper();
        Ok(instance)
    }

    /// The plugin's control instance, started if needed.
    async fn control(&self, plugin: &str) -> Result<Instance, ProviderError> {
        let now = self.now();
        if let Some((i, used)) = lock(&self.inner.runs)
            .entry(plugin.to_owned())
            .or_default()
            .control
            .as_mut()
        {
            *used = now;
            return Ok(i.clone());
        }
        let has_providers = self
            .inner
            .manager
            .get(plugin)
            .ok()
            .and_then(|v| v.manifest)
            .is_some_and(|m| !m.contributes.providers.is_empty());
        let (workspaces, probe_dir) = if has_providers {
            let dir = self.probe_dir(plugin).await?;
            (vec![dir.clone()], Some(dir))
        } else {
            (Vec::new(), None)
        };
        let config = self.instance_config(
            plugin,
            plugin,
            workspaces,
            Arc::new(SessionShared::default()),
            probe_dir,
        )?;
        let instance = self.start_instance(config).await?;
        let mut runs = lock(&self.inner.runs);
        let run = runs.entry(plugin.to_owned()).or_default();
        // Another caller may have started one meanwhile; keep the first.
        Ok(run.control.get_or_insert((instance, now)).0.clone())
    }

    fn session(&self, backend: &str, session: &str) -> Option<Arc<Session>> {
        lock(&self.inner.runs)
            .get(backend)
            .and_then(|r| r.sessions.get(session).cloned())
    }

    /// Records a failure: every instance of the plugin is discarded (their sessions
    /// resume on the next turn) and the plugin goes into backoff.
    async fn failed(&self, plugin: &str, message: &str) {
        let now = self.now();
        let cfg = &self.inner.config;
        let (instances, threads) = {
            let mut runs = lock(&self.inner.runs);
            let run = runs.entry(plugin.to_owned()).or_default();
            if run
                .last_failure
                .is_some_and(|t| now.duration_since(t) > cfg.backoff_reset)
            {
                run.failures = 0;
            }
            run.failures += 1;
            let factor = 2u32.saturating_pow(run.failures - 1);
            let backoff = cfg.backoff_base.saturating_mul(factor).min(cfg.backoff_cap);
            run.last_failure = Some(now);
            run.retry_at = Some(now + backoff);
            run.failure = Some(message.to_owned());
            tracing::warn!(plugin, failures = run.failures, backoff = ?backoff, %message, "plugin failed");
            let mut instances: Vec<Instance> =
                run.control.take().map(|(i, _)| i).into_iter().collect();
            let mut threads = Vec::new();
            for (_, s) in run.sessions.drain() {
                instances.push(s.instance.clone());
                threads.push(s.thread.clone());
            }
            (instances, threads)
        };
        for thread in threads {
            self.kernel().forget_session(&thread);
        }
        for i in instances {
            self.kernel().supervisor().kill_owner(i.owner()).await;
        }
    }

    /// Stops every instance of a plugin (disable, uninstall).
    async fn unload(&self, plugin: &str, why: &str) {
        self.clear_models_cache_for_plugin(plugin);
        let (control, sessions) = {
            let mut runs = lock(&self.inner.runs);
            let Some(run) = runs.get_mut(plugin) else {
                return;
            };
            let sessions: Vec<Arc<Session>> = run.sessions.drain().map(|(_, s)| s).collect();
            (run.control.take().map(|(i, _)| i), sessions)
        };
        for s in sessions {
            self.kernel().forget_session(&s.thread);
            let mut stopped = s.instance.stopped();
            self.drop_instance(&s.instance).await;
            drop(s);
            let _ = tokio::time::timeout(
                self.inner.config.deactivate_limit,
                stopped.wait_for(|&done| done),
            )
            .await;
        }
        if let Some(i) = control {
            let mut stopped = i.stopped();
            self.drop_instance(&i).await;
            drop(i);
            let _ = tokio::time::timeout(
                self.inner.config.deactivate_limit,
                stopped.wait_for(|&done| done),
            )
            .await;
        }
        tracing::info!(plugin, why, "plugin unloaded");
    }

    async fn drop_instance(&self, instance: &Instance) {
        let _ =
            tokio::time::timeout(self.inner.config.deactivate_limit, instance.deactivate()).await;
        self.kernel()
            .supervisor()
            .kill_owner(instance.owner())
            .await;
    }

    /// Drops instances idle as of `now`: no call in progress, no running turn, and no
    /// call for `idle_after`. Their sessions resume on the next turn. Returns how many
    /// were dropped. Runs periodically while instances exist; tests call it directly.
    pub async fn unload_idle(&self, now: Instant) -> usize {
        let idle_after = self.inner.config.idle_after;
        let mut dropped = Vec::new();
        {
            let mut runs = lock(&self.inner.runs);
            for run in runs.values_mut() {
                run.sessions.retain(|_, s| {
                    let used = *lock(&s.last_used);
                    let idle = s.busy_gen.load(Ordering::Acquire) == 0
                        && now.saturating_duration_since(used) >= idle_after;
                    if idle {
                        dropped.push((Some(s.thread.clone()), s.instance.clone()));
                    }
                    !idle
                });
                if let Some((i, used)) = &run.control
                    && now.saturating_duration_since(*used) >= idle_after
                {
                    dropped.push((None, i.clone()));
                    run.control = None;
                }
            }
        }
        let n = dropped.len();
        for (thread, instance) in dropped {
            if let Some(t) = thread {
                self.kernel().forget_session(&t);
            }
            tracing::debug!(plugin = instance.plugin(), "idle instance unloaded");
            self.drop_instance(&instance).await;
        }
        n
    }

    /// Starts the periodic idle check if it is not running; it stops when no instance
    /// is left.
    fn ensure_reaper(&self) {
        if self.inner.reaper.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        let every = (self.inner.config.idle_after / 10).max(Duration::from_secs(1));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                let Some(inner) = weak.upgrade() else { return };
                let host = PluginHost { inner };
                host.unload_idle(host.now()).await;
                let empty = lock(&host.inner.runs)
                    .values()
                    .all(|r| r.control.is_none() && r.sessions.is_empty());
                if empty {
                    host.inner.reaper.store(false, Ordering::Release);
                    return;
                }
            }
        });
    }
}

/// A provider served by a WASM plugin. Holds the host weakly: the kernel owns providers
/// and the host owns the kernel.
struct WasmProvider {
    spec: ProviderSpec,
    host: Weak<Inner>,
}

impl std::fmt::Debug for WasmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmProvider")
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

impl WasmProvider {
    fn host(&self) -> ProviderResult<PluginHost> {
        self.host
            .upgrade()
            .map(|inner| PluginHost { inner })
            .ok_or_else(|| ProviderError::new("the plugin host has stopped"))
    }

    fn pref(&self) -> wit::ProviderRef {
        wit::ProviderRef {
            id: self.spec.local.clone(),
            plugin: self.spec.contributor.clone(),
            config: self.spec.config.clone(),
        }
    }

    /// Maps a call result; a trap fails the plugin.
    async fn check<T>(
        &self,
        host: &PluginHost,
        r: Result<
            Result<T, crate::runtime::bindings::base::kumikit::host::types::Error>,
            CallError,
        >,
    ) -> ProviderResult<T> {
        match r {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(ProviderError::new(guest_message(e))),
            Err(e) => {
                if matches!(e, CallError::Trap { .. }) {
                    host.failed(&self.spec.backend, &e.to_string()).await;
                }
                Err(perr(e))
            }
        }
    }
}

impl Provider for WasmProvider {
    fn id(&self) -> &str {
        &self.spec.id
    }

    fn plugin_id(&self) -> &str {
        &self.spec.backend
    }

    fn name(&self) -> &str {
        &self.spec.name
    }

    fn contributor(&self) -> &str {
        &self.spec.contributor
    }

    fn icon(&self) -> Option<&str> {
        self.spec.icon.as_deref()
    }

    fn list_models(&self) -> BoxFuture<'_, ProviderResult<Vec<Model>>> {
        Box::pin(async move {
            let host = self.host()?;
            let slot = host.models_slot(&self.spec.id);
            let mut cached = slot.lock().await;
            let now = host.now();
            if let Some((at, models)) = cached.as_ref()
                && now.saturating_duration_since(*at) < host.inner.config.models_ttl
            {
                return Ok(models.clone());
            }
            let models = if self.spec.backend == self.spec.contributor {
                let control = host.control(&self.spec.backend).await?;
                self.check(&host, control.list_models(self.pref()).await)
                    .await?
            } else {
                // Delegated provider backend: run in a short-lived probe instance carrying
                // the declaring plugin's grants and an isolated empty probe directory, so
                // the backend can query the agent CLI for available models.
                let probe_dir = host.probe_dir(&self.spec.contributor).await?;
                let shared = Arc::new(SessionShared::default());
                let ic = host.instance_config(
                    &self.spec.backend,
                    &self.spec.contributor,
                    vec![probe_dir.clone()],
                    Arc::clone(&shared),
                    Some(probe_dir),
                )?;
                let instance = host.start_instance(ic).await?;
                let cancel_shared = Arc::clone(&shared);
                let limit = host.inner.config.models_probe_timeout;
                let watchdog = tokio::spawn(async move {
                    tokio::time::sleep(limit).await;
                    cancel_shared.cancel();
                });
                let res = self
                    .check(&host, instance.list_models(self.pref()).await)
                    .await;
                watchdog.abort();
                host.drop_instance(&instance).await;
                res?
            };
            let (models, mut options) = models;
            let out: Vec<Model> = models
                .into_iter()
                .map(|m| Model {
                    options: options.remove(&m.id).unwrap_or_default(),
                    id: m.id,
                    label: m.label,
                    capabilities: m.capabilities,
                })
                .collect();
            *cached = Some((host.now(), out.clone()));
            Ok(out)
        })
    }

    fn start_session(&self, config: SessionConfig) -> BoxFuture<'_, ProviderResult<String>> {
        Box::pin(async move {
            let host = self.host()?;
            let shared = Arc::new(SessionShared::default());
            let ic = host.instance_config(
                &self.spec.backend,
                &self.spec.contributor,
                vec![PathBuf::from(&config.workspace_root)],
                Arc::clone(&shared),
                None,
            )?;
            let instance = host.start_instance(ic).await?;
            let thread = config.thread.clone();
            let wit_config = wit::SessionConfig {
                thread: config.thread.0,
                workspace_root: config.workspace_root,
                model: config.model,
                resume: config.resume,
                mcp_server: config.mcp_server.map(|m| wit::McpServer {
                    name: m.name,
                    command: m.command,
                    args: m.args,
                    env: m.env,
                }),
                options: config.options,
            };
            let id = self
                .check(&host, instance.start_session(self.pref(), wit_config).await)
                .await?;
            let session = Arc::new(Session {
                instance,
                shared,
                thread,
                busy_gen: Arc::new(AtomicU64::new(0)),
                next_gen: AtomicU64::new(1),
                turn_lock: Arc::new(tokio::sync::Mutex::new(())),
                last_used: Mutex::new(host.now()),
            });
            {
                // The plugin may have been disabled while the session was starting; then
                // the session must not outlive the unload that already ran.
                let registered =
                    lock(&host.inner.registered).get(&self.spec.id) == Some(&self.spec);
                if registered {
                    lock(&host.inner.runs)
                        .entry(self.spec.backend.clone())
                        .or_default()
                        .sessions
                        .insert(id.clone(), Arc::clone(&session));
                    return Ok(id);
                }
            }
            host.drop_instance(&session.instance).await;
            Err(ProviderError::new(format!(
                "provider {} was removed while its session started",
                self.spec.id
            )))
        })
    }

    fn send(
        &self,
        session: String,
        input: UserInput,
        sink: EventSink,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async move {
            let host = self.host()?;
            let s = host
                .session(&self.spec.backend, &session)
                .ok_or_else(|| ProviderError::new("the provider session is no longer running"))?;
            let text = if input.attachments.is_empty() {
                input.text
            } else {
                let data_dir = host.inner.manager.dirs().plugin_data(&self.spec.backend);
                let turn_id = sink.turn().as_str().to_owned();
                materialize_attachments(data_dir, turn_id, input.text, input.attachments).await?
            };
            // Serialize turns on this session so a rapid retry after cancel waits for the
            // cancelled guest call to unwind before calling `begin_turn` (which clears
            // `cancel_pending`). Acquire the lock and mark the turn active BEFORE spawning
            // so an immediate `cancel()` never races ahead of `begin_turn` or sees
            // `busy_gen == 0`.
            let guard = Arc::clone(&s.turn_lock).lock_owned().await;
            *lock(&s.last_used) = host.now();
            let turn_gen = s.next_gen.fetch_add(1, Ordering::Relaxed);
            s.busy_gen.store(turn_gen, Ordering::Release);
            s.shared.begin_turn(sink);
            // Run the call as its own task: if the kernel drops this future (cancel), the
            // guest still finishes, or is discarded after the grace period.
            let task = {
                let s = Arc::clone(&s);
                let host = host.clone();
                tokio::spawn(async move {
                    let _guard = guard;
                    let r = s.instance.send(session, wit::UserInput { text }).await;
                    s.shared.end_turn();
                    let _ = s.busy_gen.compare_exchange(
                        turn_gen,
                        0,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    *lock(&s.last_used) = host.now();
                    r
                })
            };
            let r = task.await.map_err(perr)?;
            self.check(&host, r).await
        })
    }

    fn respond_approval(
        &self,
        session: String,
        approval: ApprovalId,
        decision: Decision,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async move {
            let host = self.host()?;
            if let Some(s) = host.session(&self.spec.backend, &session) {
                s.shared.answer(approval, decision);
            }
            Ok(())
        })
    }

    fn cancel(&self, session: String) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let Ok(host) = self.host() else { return };
            let Some(s) = host.session(&self.spec.backend, &session) else {
                return;
            };
            let cancelled_gen = s.busy_gen.load(Ordering::Acquire);
            s.shared.cancel();
            let grace = host.inner.config.cancel_grace;
            let backend = self.spec.backend.clone();
            tokio::spawn(async move {
                tokio::time::sleep(grace).await;
                if cancelled_gen == 0 || s.busy_gen.load(Ordering::Acquire) != cancelled_gen {
                    return;
                }
                tracing::warn!(plugin = %backend, "turn did not stop after cancel; discarding its instance");
                let removed = lock(&host.inner.runs)
                    .get_mut(&backend)
                    .and_then(|r| r.sessions.remove(&session));
                if removed.is_some() {
                    host.kernel().forget_session(&s.thread);
                }
                host.kernel()
                    .supervisor()
                    .kill_owner(s.instance.owner())
                    .await;
            });
        })
    }

    fn end_session(&self, session: String) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let Ok(host) = self.host() else { return };
            let removed = lock(&host.inner.runs)
                .get_mut(&self.spec.backend)
                .and_then(|r| r.sessions.remove(&session));
            let Some(s) = removed else { return };
            if s.busy_gen.load(Ordering::Acquire) != 0 {
                // Mid-turn (the core is stopping, say): calls would queue behind the
                // running `send`. Cancel the turn and kill the session's processes.
                s.shared.cancel();
                host.kernel()
                    .supervisor()
                    .kill_owner(s.instance.owner())
                    .await;
                return;
            }
            let _ = tokio::time::timeout(
                host.inner.config.deactivate_limit,
                s.instance.end_session(session),
            )
            .await;
            host.drop_instance(&s.instance).await;
        })
    }
}

/// Sanitizes an attachment filename into a safe single path component.
fn sanitize_attachment_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let trimmed = cleaned.trim_start_matches('.');
    if trimmed.is_empty() {
        "attachment".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Writes turn attachments into the provider backend's preopened `/data/attachments/<turn>`
/// directory alongside a `manifest.json`, and appends a parseable marker plus human-readable
/// fallback file list to `text` so both multimodal-aware and generic `0.2.0` providers work.
async fn materialize_attachments(
    data_dir: PathBuf,
    turn_id: String,
    text: String,
    attachments: Vec<kumikit_protocol::model::Attachment>,
) -> ProviderResult<String> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    tokio::task::spawn_blocking(move || {
        let safe_turn = sanitize_attachment_name(&turn_id);
        let dir = data_dir.join("attachments").join(&safe_turn);
        std::fs::create_dir_all(&dir)
            .map_err(|e| ProviderError::new(format!("cannot create attachment directory: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }

        let mut manifest_entries = Vec::with_capacity(attachments.len());
        let mut fallback_lines = Vec::with_capacity(attachments.len());

        for (i, a) in attachments.into_iter().enumerate() {
            let bytes = BASE64
                .decode(&a.data)
                .map_err(|e| ProviderError::new(format!("invalid attachment base64: {e}")))?;
            let filename = format!("{i}-{}", sanitize_attachment_name(&a.name));
            let host_path = dir.join(&filename);
            std::fs::write(&host_path, &bytes)
                .map_err(|e| ProviderError::new(format!("cannot write attachment: {e}")))?;
            let host_str = host_path.to_string_lossy().into_owned();
            let guest_path = format!("/data/attachments/{safe_turn}/{filename}");
            let is_image = a.mime_type.starts_with("image/");
            let dims = match (a.width, a.height) {
                (Some(w), Some(h)) => format!(", {w}x{h}"),
                _ => String::new(),
            };
            fallback_lines.push(format!(
                "- {} ({}{}): {}",
                a.name, a.mime_type, dims, host_str
            ));
            let mut entry = serde_json::json!({
                "name": a.name,
                "mimeType": a.mime_type,
                "hostPath": host_str,
                "guestPath": guest_path,
            });
            if is_image {
                entry["data"] = serde_json::Value::String(a.data);
            }
            if let Some(w) = a.width {
                entry["width"] = serde_json::Value::from(w);
            }
            if let Some(h) = a.height {
                entry["height"] = serde_json::Value::from(h);
            }
            manifest_entries.push(entry);
        }

        let manifest_host = dir.join("manifest.json");
        let manifest = serde_json::json!({ "attachments": manifest_entries });
        std::fs::write(&manifest_host, manifest.to_string())
            .map_err(|e| ProviderError::new(format!("cannot write attachment manifest: {e}")))?;

        let guest_manifest = format!("/data/attachments/{safe_turn}/manifest.json");
        let trailer = format!(
            "[kumikit-attachments:{guest_manifest}]\nAttached files:\n{}",
            fallback_lines.join("\n")
        );
        if text.is_empty() {
            Ok(trailer)
        } else {
            Ok(format!("{text}\n\n{trailer}"))
        }
    })
    .await
    .map_err(|e| ProviderError::new(format!("attachment task failed: {e}")))?
}

/// Wakes plugins for the kernel's scheduler (`docs/spec/03-kernel.md#schedules`).
struct ScheduleRunner {
    host: Weak<Inner>,
}

impl kumikit_kernel::schedule::Runner for ScheduleRunner {
    fn runnable(&self, plugin: &str) -> bool {
        let Some(inner) = self.host.upgrade() else {
            return false;
        };
        inner
            .manager
            .enabled()
            .iter()
            .any(|(r, _)| r.id == plugin && r.granted.as_ref().is_some_and(|g| g.schedules))
    }

    fn fire(
        &self,
        plugin: String,
        key: String,
        due: String,
    ) -> kumikit_kernel::provider::BoxFuture<'static, Result<(), String>> {
        let host = self.host.upgrade().map(|inner| PluginHost { inner });
        Box::pin(async move {
            let host = host.ok_or("the plugin host has stopped")?;
            tracing::info!(%plugin, %key, %due, "a schedule is due");
            let control = host.control(&plugin).await.map_err(|e| e.to_string())?;
            let r = control.fire_timer(key, due).await;
            host.guest_result(&plugin, r)
                .await
                .map_err(|e| e.to_string())
        })
    }
}
