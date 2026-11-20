//! Host bindings for the worlds the host links (WIT 0.2.0, ADR 0022). Interfaces several
//! worlds import are generated once, in `base`, and reused by the others, so one
//! `HostState` implementation serves every world.

/// `kumikit:plugin/plugin@0.2.0`.
#[allow(missing_docs, clippy::all)]
pub mod base {
    wasmtime::component::bindgen!({
        path: ["../../wit/kumikit-host@0.2.0", "../../wit/kumikit-plugin@0.2.0"],
        world: "kumikit:plugin/plugin",
        imports: {
            "kumikit:host/client.[method]subscription.subscribe": async | trappable,
            default: async,
        },
        exports: { default: async },
        with: {
            "kumikit:host/process.child": crate::runtime::host::ChildRes,
            "kumikit:host/client.subscription": crate::runtime::client::SubscriptionRes,
            "wasi:io/poll": wasmtime_wasi::p2::bindings::io::poll,
        },
    });
}

/// `kumikit:provider/plugin@0.2.0`.
#[allow(missing_docs, clippy::all)]
pub mod provider {
    wasmtime::component::bindgen!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-plugin@0.2.0",
            "../../wit/kumikit-provider@0.2.0",
        ],
        world: "kumikit:provider/plugin",
        imports: { default: async },
        exports: { default: async },
        with: {
            "kumikit:host/types": super::base::kumikit::host::types,
            "kumikit:host/log": super::base::kumikit::host::log,
            "kumikit:host/kv": super::base::kumikit::host::kv,
            "kumikit:host/process": super::base::kumikit::host::process,
            "kumikit:host/secrets": super::base::kumikit::host::secrets,
            "kumikit:host/settings": super::base::kumikit::host::settings,
            "kumikit:plugin/types": super::base::kumikit::plugin::types,
        },
    });
}

/// `kumikit:tool/plugin@0.1.0`.
#[allow(missing_docs, clippy::all)]
pub mod tool {
    wasmtime::component::bindgen!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-plugin@0.2.0",
            "../../wit/kumikit-tool@0.1.0",
        ],
        world: "kumikit:tool/plugin",
        imports: { default: async },
        exports: { default: async },
        with: {
            "kumikit:host/types": super::base::kumikit::host::types,
            "kumikit:host/log": super::base::kumikit::host::log,
            "kumikit:host/kv": super::base::kumikit::host::kv,
            "kumikit:host/process": super::base::kumikit::host::process,
            "kumikit:host/secrets": super::base::kumikit::host::secrets,
            "kumikit:host/settings": super::base::kumikit::host::settings,
            "kumikit:plugin/types": super::base::kumikit::plugin::types,
        },
    });
}
