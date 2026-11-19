#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]
//! The hello plugin loads through the pooling allocator, the compile
//! cache, the limiter and the epoch callback, and its activation is timed.

use std::time::{Duration, Instant};

use kumikit_plugin_host::engine::{self, EngineConfig, HasLimits, Limits, PluginEngine};
use wasmtime::Store;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use kumikit::host::types::{Error, ErrorCode};

wasmtime::component::bindgen!({
    path: ["../../wit/kumikit-host@0.2.0", "../../wit/kumikit-plugin@0.2.0"],
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

fn unused() -> Error {
    Error {
        code: ErrorCode::Internal,
        message: "not in this test".into(),
    }
}

impl kumikit::host::kv::Host for State {
    async fn get(&mut self, _key: String) -> Result<Option<Vec<u8>>, Error> {
        Err(unused())
    }
    async fn set(&mut self, _key: String, _value: Vec<u8>) -> Result<(), Error> {
        Err(unused())
    }
    async fn delete(&mut self, _key: String) -> Result<(), Error> {
        Err(unused())
    }
    async fn list(&mut self, _prefix: String) -> Result<Vec<String>, Error> {
        Err(unused())
    }
}
impl kumikit::host::secrets::Host for State {
    async fn get(&mut self, _name: String) -> Result<Option<String>, Error> {
        Err(unused())
    }
}
impl kumikit::host::settings::Host for State {
    async fn get(&mut self) -> Result<String, Error> {
        Err(unused())
    }
}
use kumikit::host::process::{Child, ExitStatus, SpawnRequest};
use wasmtime::component::Resource;
impl kumikit::host::process::Host for State {
    async fn spawn(&mut self, _req: SpawnRequest) -> Result<Resource<Child>, Error> {
        Err(unused())
    }
}
impl kumikit::host::process::HostChild for State {
    async fn write_stdin(&mut self, _c: Resource<Child>, _d: Vec<u8>) -> Result<(), Error> {
        Err(unused())
    }
    async fn close_stdin(&mut self, _c: Resource<Child>) {}
    async fn read_stdout(
        &mut self,
        _c: Resource<Child>,
        _m: u32,
    ) -> Result<Option<Vec<u8>>, Error> {
        Err(unused())
    }
    async fn read_stderr(
        &mut self,
        _c: Resource<Child>,
        _m: u32,
    ) -> Result<Option<Vec<u8>>, Error> {
        Err(unused())
    }
    async fn wait(&mut self, _c: Resource<Child>) -> Result<ExitStatus, Error> {
        Err(unused())
    }
    async fn kill(&mut self, _c: Resource<Child>) {}
    async fn drop(&mut self, _c: Resource<Child>) -> wasmtime::Result<()> {
        Ok(())
    }
}
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
