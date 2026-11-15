//! The wasmtime engine and per-instance limits (`docs/spec/04-plugin-host.md`,
//! execution model).
//!
//! - The pooling allocator reserves address space for a fixed number of instances; its
//!   memory slots are sized for the core's cap, and the [`Limits`] limiter enforces each
//!   plugin's own (lower) limit.
//! - The call-time limit uses epochs: a ticker advances the engine's epoch every
//!   [`EPOCH_TICK`], each store's deadline is one tick ahead, and the deadline callback
//!   traps once the guest has run longer than its limit since it last entered wasm or
//!   returned from a host import (recorded by the store's call hook). The ticker runs
//!   only while instances exist.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use wasmtime::{
    CallHook, Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, ResourceLimiter,
    Store, UpdateDeadline,
};

/// How often the epoch advances.
pub const EPOCH_TICK: Duration = Duration::from_millis(100);

/// Default linear memory per instance.
pub const DEFAULT_MEMORY: usize = 64 << 20;
/// The core's cap on linear memory per instance; manifests cannot ask for more.
pub const MEMORY_CAP: usize = 256 << 20;
/// Default guest time per call.
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// The core's cap on guest time per call.
pub const CALL_TIMEOUT_CAP: Duration = Duration::from_secs(60);

/// Engine settings that tests change.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Component instances that can exist at once (pooling slots).
    pub instance_slots: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig { instance_slots: 64 }
    }
}

/// Failure to set up the engine.
#[derive(Debug, thiserror::Error)]
#[error("cannot start the WASM engine: {0}")]
pub struct EngineError(String);

/// The shared engine plus the epoch ticker.
#[derive(Debug, Clone)]
pub struct PluginEngine {
    engine: Engine,
    ticker: Arc<Ticker>,
}

impl PluginEngine {
    /// Creates the engine. `cache_dir` holds compiled components between runs.
    pub fn new(config: &EngineConfig, cache_dir: &Path) -> Result<Self, EngineError> {
        let mut c = Config::new();
        c.wasm_component_model(true);
        c.epoch_interruption(true);

        let slots = config.instance_slots;
        let mut pool = PoolingAllocationConfig::default();
        // A Rust wasm32-wasip2 component has a few core instances, memories and tables.
        pool.total_component_instances(slots)
            .total_core_instances(slots * 8)
            .total_memories(slots * 2)
            .total_tables(slots * 4)
            .total_stacks(slots)
            .max_memory_size(MEMORY_CAP)
            .max_memories_per_component(2)
            // Return memory to the OS when an instance goes away.
            .linear_memory_keep_resident(0);
        c.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));
        // Reserve only the cap per memory (plus guard) instead of 4 GiB, so the pool
        // costs tens of GB of address space, not hundreds. Bounds checks are explicit.
        c.memory_reservation(MEMORY_CAP as u64);

        let mut cache = wasmtime::CacheConfig::new();
        cache.with_directory(cache_dir);
        let cache = wasmtime::Cache::new(cache).map_err(|e| EngineError(e.to_string()))?;
        c.cache(Some(cache));

        let engine = Engine::new(&c).map_err(|e| EngineError(e.to_string()))?;
        Ok(PluginEngine {
            ticker: Ticker::new(engine.weak()),
            engine,
        })
    }

    /// The wasmtime engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Keeps the epoch ticker running while the returned guard lives.
    pub fn ticking(&self) -> TickGuard {
        self.ticker.acquire()
    }
}

/// Data every store carries for the limiter and the call-time limit.
#[derive(Debug)]
pub struct Limits {
    memory: usize,
    call_timeout: Duration,
    /// When the guest last started running (entered wasm or returned from the host).
    guest_since: Instant,
    /// Whether the last trap was the call-time limit.
    timed_out: bool,
}

impl Limits {
    /// Limits for one instance.
    pub fn new(memory: usize, call_timeout: Duration) -> Self {
        Limits {
            memory: memory.min(MEMORY_CAP),
            call_timeout: call_timeout.min(CALL_TIMEOUT_CAP),
            guest_since: Instant::now(),
            timed_out: false,
        }
    }

    /// Whether the call-time limit stopped the guest.
    pub fn timed_out(&self) -> bool {
        self.timed_out
    }
}

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= self.memory)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= 100_000)
    }
}

/// Store data that carries [`Limits`].
pub trait HasLimits: Send + 'static {
    /// The limits.
    fn limits(&mut self) -> &mut Limits;
}

/// Installs the limiter, the call hook and the epoch deadline on a new store.
pub fn apply_limits<T: HasLimits>(store: &mut Store<T>) {
    store.limiter(|data| data.limits());
    store.call_hook(|mut state, hook| {
        if matches!(hook, CallHook::CallingWasm | CallHook::ReturningFromHost) {
            state.data_mut().limits().guest_since = Instant::now();
        }
        Ok(())
    });
    store.set_epoch_deadline(1);
    store.epoch_deadline_callback(|mut state| {
        let limits = state.data_mut().limits();
        if limits.guest_since.elapsed() > limits.call_timeout {
            limits.timed_out = true;
            Ok(UpdateDeadline::Interrupt)
        } else {
            // Give other tasks a turn on this thread; check again next tick.
            Ok(UpdateDeadline::Yield(1))
        }
    });
}

/// Advances the epoch every [`EPOCH_TICK`] while at least one guard exists.
///
/// If the last guard drops and a new one is taken before the old thread notices, two
/// threads tick for a moment. That is harmless: the deadline callback measures elapsed
/// time, so extra ticks only mean more frequent checks.
struct Ticker {
    engine: wasmtime::EngineWeak,
    users: AtomicUsize,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    me: Weak<Ticker>,
}

impl std::fmt::Debug for Ticker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ticker")
            .field("users", &self.users.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Ticker {
    fn new(engine: wasmtime::EngineWeak) -> Arc<Self> {
        Arc::new_cyclic(|me| Ticker {
            engine,
            users: AtomicUsize::new(0),
            thread: Mutex::new(None),
            me: me.clone(),
        })
    }

    fn acquire(&self) -> TickGuard {
        let mut thread = self.thread.lock().unwrap_or_else(|p| p.into_inner());
        if self.users.fetch_add(1, Ordering::AcqRel) == 0 {
            let me = self.me.clone();
            let engine = self.engine.clone();
            *thread = Some(std::thread::spawn(move || {
                loop {
                    std::thread::sleep(EPOCH_TICK);
                    let Some(ticker) = me.upgrade() else { return };
                    if ticker.users.load(Ordering::Acquire) == 0 {
                        return;
                    }
                    match engine.upgrade() {
                        Some(e) => e.increment_epoch(),
                        None => return,
                    }
                }
            }));
        }
        TickGuard {
            ticker: self.me.clone(),
        }
    }
}

/// Keeps the epoch ticker running; see [`PluginEngine::ticking`].
#[derive(Debug)]
pub struct TickGuard {
    ticker: Weak<Ticker>,
}

impl Drop for TickGuard {
    fn drop(&mut self) {
        if let Some(t) = self.ticker.upgrade() {
            t.users.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
