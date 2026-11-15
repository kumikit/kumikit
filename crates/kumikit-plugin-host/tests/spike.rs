#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]
//! Step 2 spike: the hello plugin loads through the pooling allocator, the compile
//! cache, the limiter and the epoch callback, and its activation is timed.

use std::time::{Duration, Instant};

use kumikit_plugin_host::engine::{self, EngineConfig, HasLimits, Limits, PluginEngine};
use wasmtime::Store;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: ["../../wit/kumikit-host@0.1.0", "../../wit/kumikit-plugin@0.1.0"],
    world: "kumikit:plugin/plugin",
    imports: { default: async },
    exports: { default: async },
});

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: Limits,
    logged: Vec<String>,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl HasLimits for State {
    fn limits(&mut self) -> &mut Limits {
        &mut self.limits
    }
}

impl kumikit::host::types::Host for State {}
impl kumikit::host::log::Host for State {
    async fn log(&mut self, _level: kumikit::host::log::Level, message: String) {
        self.logged.push(message);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_activates_from_the_cache() {
    let cache = tempfile::tempdir().unwrap();
    let engine = PluginEngine::new(&EngineConfig::default(), cache.path()).unwrap();
    let bytes = std::fs::read(kumikit_test_plugins::HELLO_WASM).unwrap();

    let t = Instant::now();
    let _ = Component::new(engine.engine(), &bytes).unwrap();
    let compile = t.elapsed();

    let mut linker = Linker::<State>::new(engine.engine());
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
    Plugin::add_to_linker::<State, wasmtime::component::HasSelf<State>>(&mut linker, |s| s)
        .unwrap();
    let _tick = engine.ticking();

    let mut times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let component = Component::new(engine.engine(), &bytes).unwrap();
        let mut store = Store::new(
            engine.engine(),
            State {
                wasi: WasiCtx::builder().build(),
                table: ResourceTable::new(),
                limits: Limits::new(engine::DEFAULT_MEMORY, engine::DEFAULT_CALL_TIMEOUT),
                logged: Vec::new(),
            },
        );
        engine::apply_limits(&mut store);
        let plugin = Plugin::instantiate_async(&mut store, &component, &linker)
            .await
            .unwrap();
        plugin
            .kumikit_plugin_lifecycle()
            .call_activate(&mut store)
            .await
            .unwrap()
            .unwrap();
        times.push(t.elapsed());
        assert_eq!(store.data().logged, ["hello activated"]);
    }
    times.sort();
    eprintln!(
        "compile {compile:?}, activation from cache (median of 5) {:?}",
        times[2]
    );
    assert!(
        times[2] < Duration::from_millis(50),
        "activation took {:?}",
        times[2]
    );
}
