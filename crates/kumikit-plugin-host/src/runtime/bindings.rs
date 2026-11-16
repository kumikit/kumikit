//! Host bindings for the two worlds. Interfaces both worlds import are generated once,
//! in `base`, and reused by `provider`, so one `HostState` implementation serves both.

/// `kumikit:plugin/plugin@0.1.0`.
#[allow(missing_docs, clippy::all)]
pub mod base {
    wasmtime::component::bindgen!({
        path: ["../../wit/kumikit-host@0.1.0", "../../wit/kumikit-plugin@0.1.0"],
        world: "kumikit:plugin/plugin",
        imports: { default: async },
        exports: { default: async },
    });
}

/// `kumikit:provider/plugin@0.1.0`.
#[allow(missing_docs, clippy::all)]
pub mod provider {
    wasmtime::component::bindgen!({
        path: [
            "../../wit/kumikit-host@0.1.0",
            "../../wit/kumikit-plugin@0.1.0",
            "../../wit/kumikit-provider@0.1.0",
        ],
        world: "kumikit:provider/plugin",
        imports: { default: async },
        exports: { default: async },
        with: {
            "kumikit:host/types": super::base::kumikit::host::types,
            "kumikit:host/log": super::base::kumikit::host::log,
            "kumikit:host/kv": super::base::kumikit::host::kv,
            "kumikit:host/process.child": crate::runtime::host::ChildRes,
        },
    });
}
