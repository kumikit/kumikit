//! Running components (`docs/spec/04-plugin-host.md#execution-model`).
//!
//! - [`Runtime`] creates the engine and linkers on first use, so a core whose plugins
//!   are never used does no WASM work at all.
//! - [`Runtime::prepare`] compiles a component (into the cache) and checks that its
//!   imports can be satisfied and its exports match its world; enabling a plugin calls
//!   it, so problems show at install time, and activation loads compiled code.
//! - [`Instance`] is one component instance driven by its own task: calls are sent to
//!   it and run one at a time. A caller that stops waiting never interrupts a guest
//!   mid-call; a trap or limit breach ends the instance and kills its processes.

pub mod bindings;
pub mod host;
pub mod outbound;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use kumikit_kernel::Kernel;
use kumikit_kernel::process::OwnerId;
use tokio::sync::{mpsc, oneshot};
use wasmtime::Store;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime_wasi::{FsPerms, WasiCtx};

use crate::engine::{self, EngineConfig, Limits, PluginEngine, TickGuard};
use crate::manifest::{WORLD_PLUGIN, WORLD_PROVIDER};
use bindings::base::kumikit::host::types::Error as GuestError;
pub use bindings::base::kumikit::plugin::types::{CallContext, CommandResult, WorkspaceRef};
use bindings::provider::kumikit::provider::types::{Model, ProviderRef, SessionConfig, UserInput};
pub use host::{HostState, Identity, SessionShared};

/// Where a guest's data directory appears inside the sandbox.
pub const GUEST_DATA_DIR: &str = "/data";

/// A failure to compile or link a component.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct PrepareError(pub String);

/// Why a call into an instance did not return a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallError {
    /// The guest trapped or broke a limit; the instance is gone.
    #[error("plugin {plugin} failed: {message}")]
    Trap {
        /// The plugin.
        plugin: String,
        /// What happened.
        message: String,
        /// Whether the call-time limit stopped it.
        timed_out: bool,
    },
    /// The instance ended earlier (after a trap, or it was stopped).
    #[error("plugin instance is no longer running")]
    Gone,
    /// The plugin does not implement the world this call needs.
    #[error("plugin does not implement {0}")]
    WrongWorld(&'static str),
}

/// A compiled, linked component, ready to instantiate.
#[derive(Clone)]
pub enum Prepared {
    /// Base world.
    Base(bindings::base::PluginPre<HostState>),
    /// Provider world.
    Provider(bindings::provider::PluginPre<HostState>),
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Prepared::Base(_) => "Prepared::Base",
            Prepared::Provider(_) => "Prepared::Provider",
        })
    }
}

struct Linkers {
    engine: PluginEngine,
    base: Linker<HostState>,
    provider: Linker<HostState>,
}

/// The lazily started WASM runtime.
pub struct Runtime {
    config: EngineConfig,
    cache_dir: PathBuf,
    linkers: OnceLock<Result<Linkers, String>>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("started", &self.is_started())
            .finish_non_exhaustive()
    }
}

fn wasi_linker(engine: &wasmtime::Engine) -> wasmtime::Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)?;
    Ok(linker)
}

impl Runtime {
    /// A runtime that will keep compiled code in `cache_dir`. Starts nothing.
    pub fn new(config: EngineConfig, cache_dir: impl Into<PathBuf>) -> Self {
        Runtime {
            config,
            cache_dir: cache_dir.into(),
            linkers: OnceLock::new(),
        }
    }

    /// Whether the engine exists yet.
    pub fn is_started(&self) -> bool {
        self.linkers.get().is_some()
    }

    fn linkers(&self) -> Result<&Linkers, PrepareError> {
        self.linkers
            .get_or_init(|| {
                std::fs::create_dir_all(&self.cache_dir).map_err(|e| e.to_string())?;
                let engine =
                    PluginEngine::new(&self.config, &self.cache_dir).map_err(|e| e.to_string())?;
                let mut base = wasi_linker(engine.engine()).map_err(|e| e.to_string())?;
                bindings::base::Plugin::add_to_linker::<HostState, HasSelf<HostState>>(
                    &mut base,
                    |s| s,
                )
                .map_err(|e| e.to_string())?;
                let mut provider = wasi_linker(engine.engine()).map_err(|e| e.to_string())?;
                bindings::provider::Plugin::add_to_linker::<HostState, HasSelf<HostState>>(
                    &mut provider,
                    |s| s,
                )
                .map_err(|e| e.to_string())?;
                tracing::debug!("WASM engine started");
                Ok(Linkers {
                    engine,
                    base,
                    provider,
                })
            })
            .as_ref()
            .map_err(|e| PrepareError(format!("cannot start the WASM engine: {e}")))
    }

    /// Compiles the component at `wasm` and links it for `world`. Compilation runs on
    /// the blocking pool and fills the cache, so later calls are fast.
    pub async fn prepare(&self, wasm: &Path, world: &str) -> Result<Prepared, PrepareError> {
        let linkers = self.linkers()?;
        let engine = linkers.engine.engine().clone();
        let path = wasm.to_path_buf();
        let component = tokio::task::spawn_blocking(move || Component::from_file(&engine, &path))
            .await
            .map_err(|e| PrepareError(e.to_string()))?
            .map_err(|e| PrepareError(format!("not a valid component: {e:#}")))?;
        let unsupported = |e: wasmtime::Error| {
            PrepareError(format!(
                "the component needs something this core does not provide: {e:#}"
            ))
        };
        match world {
            WORLD_PLUGIN => {
                let pre = linkers
                    .base
                    .instantiate_pre(&component)
                    .map_err(unsupported)?;
                bindings::base::PluginPre::new(pre)
                    .map(Prepared::Base)
                    .map_err(|e| PrepareError(format!("does not implement {WORLD_PLUGIN}: {e:#}")))
            }
            WORLD_PROVIDER => {
                let pre = linkers
                    .provider
                    .instantiate_pre(&component)
                    .map_err(unsupported)?;
                bindings::provider::PluginPre::new(pre)
                    .map(Prepared::Provider)
                    .map_err(|e| {
                        PrepareError(format!("does not implement {WORLD_PROVIDER}: {e:#}"))
                    })
            }
            other => Err(PrepareError(format!("world {other} is not supported"))),
        }
    }

    fn engine(&self) -> Result<&PluginEngine, PrepareError> {
        Ok(&self.linkers()?.engine)
    }
}

/// How to start an instance.
#[derive(Debug, Clone)]
pub struct InstanceConfig {
    /// Who it is and what it may do.
    pub identity: Identity,
    /// Its private data directory on the host (created if missing).
    pub data_dir: PathBuf,
    /// Linear memory limit, bytes.
    pub memory: usize,
    /// Guest time per call.
    pub call_timeout: std::time::Duration,
    /// Session state shared with the host side of a provider.
    pub session: Arc<SessionShared>,
    /// The plugin's settings schema file, if it has one (`kumikit:host/settings`).
    pub settings_schema: Option<PathBuf>,
}

enum Bindings {
    Base(bindings::base::Plugin),
    Provider(bindings::provider::Plugin),
}

type Job = Box<
    dyn for<'a> FnOnce(
            &'a mut Store<HostState>,
            &'a Bindings,
        ) -> std::pin::Pin<Box<dyn Future<Output = bool> + Send + 'a>>
        + Send,
>;

/// One running component instance. Dropping the last handle stops its task and kills
/// its processes.
#[derive(Debug, Clone)]
pub struct Instance {
    inner: Arc<InstanceInner>,
}

#[derive(Debug)]
struct InstanceInner {
    jobs: mpsc::Sender<Job>,
    plugin: String,
    owner: OwnerId,
    kernel: Kernel,
}

impl Drop for InstanceInner {
    fn drop(&mut self) {
        let (kernel, owner) = (self.kernel.clone(), self.owner.clone());
        // Kill in the background; the runtime may be shutting down, in which case the
        // trees are killed by their handles' drops anyway.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { kernel.supervisor().kill_owner(&owner).await });
        }
    }
}

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

fn trap(plugin: &str, store: &Store<HostState>, e: &wasmtime::Error) -> CallError {
    let timed_out = store.data().limits.timed_out();
    let message = if timed_out {
        "it ran longer than its call time limit without returning to the host".to_owned()
    } else {
        format!("{e:#}")
    };
    CallError::Trap {
        plugin: plugin.to_owned(),
        message,
        timed_out,
    }
}

impl Instance {
    /// Instantiates `prepared` and starts its task. Does not call `activate`.
    pub async fn start(
        runtime: &Runtime,
        prepared: &Prepared,
        config: InstanceConfig,
        kernel: Kernel,
    ) -> Result<Instance, CallError> {
        let plugin = config.identity.plugin.clone();
        let failed = |message: String| CallError::Trap {
            plugin: plugin.clone(),
            message,
            timed_out: false,
        };
        let engine = runtime.engine().map_err(|e| failed(e.0))?;
        let data_dir = config.data_dir.clone();
        tokio::task::spawn_blocking(move || std::fs::create_dir_all(&data_dir))
            .await
            .map_err(|e| failed(e.to_string()))?
            .map_err(|e| failed(format!("cannot create the data directory: {e}")))?;
        let mut wasi = WasiCtx::builder();
        wasi.stdout(host::LogStream {
            plugin: plugin.clone(),
            stream: "stdout",
        })
        .stderr(host::LogStream {
            plugin: plugin.clone(),
            stream: "stderr",
        })
        .allow_tcp(false)
        .allow_udp(false)
        .allow_ip_name_lookup(false)
        .socket_addr_check(|_, _| Box::pin(async { false }))
        .preopened_dir(&config.data_dir, GUEST_DATA_DIR, FsPerms::ReadWrite)
        .map_err(|e| failed(format!("cannot open the data directory: {e:#}")))?;
        let owner = OwnerId(format!(
            "{plugin}#{}",
            NEXT_OWNER.fetch_add(1, Ordering::Relaxed)
        ));
        let state = HostState {
            wasi: wasi.build(),
            http: wasmtime_wasi_http::WasiHttpCtx::new(),
            outbound: outbound::Outbound {
                plugin: config.identity.plugin.clone(),
            },
            table: ResourceTable::new(),
            limits: Limits::new(config.memory, config.call_timeout),
            kernel: kernel.clone(),
            id: config.identity,
            owner: owner.clone(),
            session: config.session,
            settings_schema: config.settings_schema,
        };
        let mut store = Store::new(engine.engine(), state);
        engine::apply_limits(&mut store);
        let tick = engine.ticking();
        let bindings = match prepared {
            Prepared::Base(pre) => pre.instantiate_async(&mut store).await.map(Bindings::Base),
            Prepared::Provider(pre) => pre
                .instantiate_async(&mut store)
                .await
                .map(Bindings::Provider),
        }
        .map_err(|e| trap(&plugin, &store, &e))?;

        let (jobs, rx) = mpsc::channel::<Job>(8);
        tokio::spawn(run(store, bindings, rx, tick));
        Ok(Instance {
            inner: Arc::new(InstanceInner {
                jobs,
                plugin,
                owner,
                kernel,
            }),
        })
    }

    /// The supervisor owner of this instance's processes.
    pub fn owner(&self) -> &OwnerId {
        &self.inner.owner
    }

    /// The plugin whose code runs.
    pub fn plugin(&self) -> &str {
        &self.inner.plugin
    }

    /// Runs `f` on the instance's task. `f` returns the value and whether the instance
    /// is still usable.
    async fn call<T: Send + 'static>(
        &self,
        f: impl for<'a> FnOnce(
            &'a mut Store<HostState>,
            &'a Bindings,
        ) -> std::pin::Pin<
            Box<dyn Future<Output = (Result<T, CallError>, bool)> + Send + 'a>,
        > + Send
        + 'static,
    ) -> Result<T, CallError> {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move |store, b| {
            Box::pin(async move {
                let (result, alive) = f(store, b).await;
                let _ = tx.send(result);
                alive
            })
        });
        self.inner
            .jobs
            .send(job)
            .await
            .map_err(|_| CallError::Gone)?;
        rx.await.map_err(|_| CallError::Gone)?
    }

    /// Calls `activate`.
    pub async fn activate(&self) -> Result<Result<(), GuestError>, CallError> {
        let plugin = self.inner.plugin.clone();
        self.call(move |store, b| {
            Box::pin(async move {
                let r = match b {
                    Bindings::Base(p) => {
                        p.kumikit_plugin_lifecycle()
                            .call_activate(&mut *store)
                            .await
                    }
                    Bindings::Provider(p) => {
                        p.kumikit_plugin_lifecycle()
                            .call_activate(&mut *store)
                            .await
                    }
                };
                finish(&plugin, store, r)
            })
        })
        .await
    }

    /// Calls `deactivate`.
    pub async fn deactivate(&self) -> Result<(), CallError> {
        let plugin = self.inner.plugin.clone();
        self.call(move |store, b| {
            Box::pin(async move {
                let r = match b {
                    Bindings::Base(p) => {
                        p.kumikit_plugin_lifecycle()
                            .call_deactivate(&mut *store)
                            .await
                    }
                    Bindings::Provider(p) => {
                        p.kumikit_plugin_lifecycle()
                            .call_deactivate(&mut *store)
                            .await
                    }
                };
                finish(&plugin, store, r)
            })
        })
        .await
    }

    /// Calls `rpc.handle`, acting for `workspace` (its root) during the call.
    pub async fn handle_rpc(
        &self,
        method: String,
        params: String,
        ctx: CallContext,
    ) -> Result<Result<String, GuestError>, CallError> {
        let plugin = self.inner.plugin.clone();
        self.call(move |store, b| {
            Box::pin(async move {
                let acting = act_for(store, &ctx);
                let r = match b {
                    Bindings::Base(p) => {
                        p.kumikit_plugin_rpc()
                            .call_handle(&mut *store, &method, &params, &ctx)
                            .await
                    }
                    Bindings::Provider(p) => {
                        p.kumikit_plugin_rpc()
                            .call_handle(&mut *store, &method, &params, &ctx)
                            .await
                    }
                };
                store.data_mut().id.workspaces = acting;
                finish(&plugin, store, r)
            })
        })
        .await
    }

    /// Calls `commands.run`, acting for the context's workspace during the call.
    pub async fn run_command(
        &self,
        id: String,
        args: String,
        ctx: CallContext,
    ) -> Result<Result<CommandResult, GuestError>, CallError> {
        let plugin = self.inner.plugin.clone();
        self.call(move |store, b| {
            Box::pin(async move {
                let acting = act_for(store, &ctx);
                let r = match b {
                    Bindings::Base(p) => {
                        p.kumikit_plugin_commands()
                            .call_run(&mut *store, &id, &args, &ctx)
                            .await
                    }
                    Bindings::Provider(p) => {
                        p.kumikit_plugin_commands()
                            .call_run(&mut *store, &id, &args, &ctx)
                            .await
                    }
                };
                store.data_mut().id.workspaces = acting;
                finish(&plugin, store, r)
            })
        })
        .await
    }

    /// Calls `timers.fire`.
    pub async fn fire_timer(
        &self,
        key: String,
        due: String,
    ) -> Result<Result<(), GuestError>, CallError> {
        let plugin = self.inner.plugin.clone();
        self.call(move |store, b| {
            Box::pin(async move {
                let r = match b {
                    Bindings::Base(p) => {
                        p.kumikit_plugin_timers()
                            .call_fire(&mut *store, &key, &due)
                            .await
                    }
                    Bindings::Provider(p) => {
                        p.kumikit_plugin_timers()
                            .call_fire(&mut *store, &key, &due)
                            .await
                    }
                };
                finish(&plugin, store, r)
            })
        })
        .await
    }

    /// Calls `list-models`.
    pub async fn list_models(
        &self,
        provider: ProviderRef,
    ) -> Result<Result<Vec<Model>, GuestError>, CallError> {
        let plugin = self.inner.plugin.clone();
        self.call(move |store, b| {
            Box::pin(async move {
                let Bindings::Provider(p) = b else {
                    return (Err(CallError::WrongWorld(WORLD_PROVIDER)), true);
                };
                let r = p
                    .kumikit_provider_provider()
                    .call_list_models(&mut *store, &provider)
                    .await;
                finish(&plugin, store, r)
            })
        })
        .await
    }

    /// Calls `start-session`.
    pub async fn start_session(
        &self,
        provider: ProviderRef,
        config: SessionConfig,
    ) -> Result<Result<String, GuestError>, CallError> {
        let plugin = self.inner.plugin.clone();
        self.call(move |store, b| {
            Box::pin(async move {
                let Bindings::Provider(p) = b else {
                    return (Err(CallError::WrongWorld(WORLD_PROVIDER)), true);
                };
                let r = p
                    .kumikit_provider_provider()
                    .call_start_session(&mut *store, &provider, &config)
                    .await;
                finish(&plugin, store, r)
            })
        })
        .await
    }

    /// Calls `send`; returns when the guest's turn is over.
    pub async fn send(
        &self,
        session: String,
        input: UserInput,
    ) -> Result<Result<(), GuestError>, CallError> {
        let plugin = self.inner.plugin.clone();
        self.call(move |store, b| {
            Box::pin(async move {
                let Bindings::Provider(p) = b else {
                    return (Err(CallError::WrongWorld(WORLD_PROVIDER)), true);
                };
                let r = p
                    .kumikit_provider_provider()
                    .call_send(&mut *store, &session, &input)
                    .await;
                finish(&plugin, store, r)
            })
        })
        .await
    }

    /// Calls `end-session`.
    pub async fn end_session(&self, session: String) -> Result<(), CallError> {
        let plugin = self.inner.plugin.clone();
        self.call(move |store, b| {
            Box::pin(async move {
                let Bindings::Provider(p) = b else {
                    return (Err(CallError::WrongWorld(WORLD_PROVIDER)), true);
                };
                let r = p
                    .kumikit_provider_provider()
                    .call_end_session(&mut *store, &session)
                    .await;
                finish(&plugin, store, r)
            })
        })
        .await
    }
}

/// Turns a call's outcome into the caller's result and whether the instance survives.
/// Makes the instance act for the context's workspace (or none) for one call; returns
/// what it acted for before, to restore afterwards.
fn act_for(store: &mut Store<HostState>, ctx: &CallContext) -> Vec<PathBuf> {
    let now = ctx
        .workspace
        .as_ref()
        .map(|w| vec![PathBuf::from(&w.root)])
        .unwrap_or_default();
    std::mem::replace(&mut store.data_mut().id.workspaces, now)
}

fn finish<T>(
    plugin: &str,
    store: &Store<HostState>,
    r: wasmtime::Result<T>,
) -> (Result<T, CallError>, bool) {
    match r {
        Ok(v) => (Ok(v), true),
        Err(e) => {
            let e = trap(plugin, store, &e);
            tracing::warn!(plugin, error = %e, "plugin trapped");
            (Err(e), false)
        }
    }
}

/// The instance's task: runs jobs until a trap, or until every handle is gone.
async fn run(
    mut store: Store<HostState>,
    bindings: Bindings,
    mut jobs: mpsc::Receiver<Job>,
    _tick: TickGuard,
) {
    while let Some(job) = jobs.recv().await {
        if !job(&mut store, &bindings).await {
            break;
        }
    }
    let owner = store.data().owner.clone();
    let kernel = store.data().kernel.clone();
    drop(store);
    kernel.supervisor().kill_owner(&owner).await;
}
