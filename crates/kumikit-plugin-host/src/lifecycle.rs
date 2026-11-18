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
use std::sync::atomic::{AtomicBool, Ordering};
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
use crate::runtime::{CallError, Identity, Instance, InstanceConfig, SessionShared};

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
}

/// The qualified id of a provider.
pub fn qualified(plugin: &str, local: &str) -> String {
    format!("{plugin}/{local}")
}

struct Session {
    instance: Instance,
    shared: Arc<SessionShared>,
    thread: ThreadId,
    busy: Arc<AtomicBool>,
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

struct Inner {
    manager: Manager,
    config: HostConfig,
    runs: Mutex<HashMap<String, PluginRun>>,
    registered: Mutex<BTreeMap<String, ProviderSpec>>,
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
        self.inner.manager.uninstall(id, delete_data).await?;
        self.reconcile().await;
        if was_enabled {
            self.inner.changed.send_modify(|v| *v += 1);
        }
        self.unload(id, "removed").await;
        lock(&self.inner.runs).remove(id);
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
                let spec = match entry.backend() {
                    None => ProviderSpec {
                        id: id.clone(),
                        local: entry.id.clone(),
                        name: entry.name.clone(),
                        contributor: record.id.clone(),
                        backend: record.id.clone(),
                        config: None,
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
            }
        }
        for (id, spec) in wanted {
            if current.get(&id) == Some(&spec) {
                continue;
            }
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

    fn instance_config(
        &self,
        backend: &str,
        grants_from: &str,
        workspaces: Vec<PathBuf>,
        session: Arc<SessionShared>,
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
        let config = self.instance_config(
            plugin,
            plugin,
            Vec::new(),
            Arc::new(SessionShared::default()),
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
            self.drop_instance(&s.instance).await;
        }
        if let Some(i) = control {
            self.drop_instance(&i).await;
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
                    let idle = !s.busy.load(Ordering::Acquire)
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
            Ok(Err(e)) => Err(ProviderError::new(e.message)),
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

    fn list_models(&self) -> BoxFuture<'_, ProviderResult<Vec<Model>>> {
        Box::pin(async move {
            let host = self.host()?;
            let control = host.control(&self.spec.backend).await?;
            let models = self
                .check(&host, control.list_models(self.pref()).await)
                .await?;
            Ok(models
                .into_iter()
                .map(|m| Model {
                    id: m.id,
                    label: m.label,
                    capabilities: m.capabilities,
                })
                .collect())
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
            )?;
            let instance = host.start_instance(ic).await?;
            let thread = config.thread.clone();
            let wit_config = wit::SessionConfig {
                thread: config.thread.0,
                workspace_root: config.workspace_root,
                model: config.model,
                resume: config.resume,
                mcp_endpoint: config.mcp_endpoint,
                options: config.options,
            };
            let id = self
                .check(&host, instance.start_session(self.pref(), wit_config).await)
                .await?;
            let session = Arc::new(Session {
                instance,
                shared,
                thread,
                busy: Arc::new(AtomicBool::new(false)),
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
            *lock(&s.last_used) = host.now();
            s.busy.store(true, Ordering::Release);
            s.shared.begin_turn(sink);
            // Run the call as its own task: if the kernel drops this future (cancel), the
            // guest still finishes, or is discarded after the grace period.
            let task = {
                let s = Arc::clone(&s);
                let host = host.clone();
                tokio::spawn(async move {
                    let r = s
                        .instance
                        .send(session, wit::UserInput { text: input.text })
                        .await;
                    s.shared.end_turn();
                    s.busy.store(false, Ordering::Release);
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
            s.shared.cancel();
            let grace = host.inner.config.cancel_grace;
            let backend = self.spec.backend.clone();
            tokio::spawn(async move {
                tokio::time::sleep(grace).await;
                if !s.busy.load(Ordering::Acquire) {
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
            if s.busy.load(Ordering::Acquire) {
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
