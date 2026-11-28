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

/// `kumikit:turn/steering@0.1.0`: the steering import (ADR 0028), linked for every
/// provider; providers built for `kumikit:turn/provider@0.1.0` use it.
#[allow(missing_docs, clippy::all)]
pub mod turn {
    wasmtime::component::bindgen!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-plugin@0.2.0",
            "../../wit/kumikit-provider@0.2.0",
            "../../wit/kumikit-turn@0.1.0",
        ],
        world: "kumikit:turn/steering",
        imports: { default: async },
        with: {
            "kumikit:host/types": super::base::kumikit::host::types,
            "kumikit:host/process": super::base::kumikit::host::process,
            "kumikit:provider/types": super::provider::kumikit::provider::types,
        },
    });
}

/// `kumikit:turn/client-steering@0.1.0`: client-side steering import (ADR 0028), linked
/// for base and tool plugins.
#[allow(missing_docs, clippy::all)]
pub mod turn_client {
    wasmtime::component::bindgen!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-plugin@0.2.0",
            "../../wit/kumikit-provider@0.2.0",
            "../../wit/kumikit-turn@0.1.0",
        ],
        world: "kumikit:turn/client-steering",
        imports: { default: async },
        with: {
            "kumikit:host/types": super::base::kumikit::host::types,
        },
    });
}

/// `kumikit:threads/client-threads@0.1.0`: additional client operations for threads
/// (defer, queue, read-events, suggest-title; ADR 0027, ADR 0030), linked for base and
/// tool plugins.
#[allow(missing_docs, clippy::all)]
pub mod threads_client {
    wasmtime::component::bindgen!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-threads@0.1.0",
        ],
        world: "kumikit:threads/client-threads",
        imports: { default: async },
        with: {
            "kumikit:host/types": super::base::kumikit::host::types,
        },
    });
}

/// `kumikit:threads/client-threads@0.2.0`: 0.1.0 plus `list-models` and `create-thread`
/// with model settings and a bounded permission mode (ADR 0034), linked for base and
/// tool plugins alongside 0.1.0.
#[allow(missing_docs, clippy::all)]
pub mod threads_client_v2 {
    wasmtime::component::bindgen!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-threads@0.2.0",
        ],
        world: "kumikit:threads/client-threads",
        imports: { default: async },
        with: {
            "kumikit:host/types": super::base::kumikit::host::types,
            "kumikit:host/client": super::base::kumikit::host::client,
        },
    });
}

/// `kumikit:session/provider-events@0.1.0`: additive provider session events
/// (`commands.available` and `limit.reached`, M7 steps 7-8), linked for every provider.
#[allow(missing_docs, clippy::all)]
pub mod session_events {
    wasmtime::component::bindgen!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-session@0.1.0",
        ],
        world: "kumikit:session/provider-events",
        imports: { default: async },
        with: {
            "kumikit:host/types": super::base::kumikit::host::types,
        },
    });
}

/// `kumikit:models/provider-models@0.1.0`: model settings a provider reports during
/// `list-models` (ADR 0033), linked for every provider.
#[allow(missing_docs, clippy::all)]
pub mod models {
    wasmtime::component::bindgen!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-models@0.1.0",
        ],
        world: "kumikit:models/provider-models",
        imports: { default: async },
        with: {
            "kumikit:host/types": super::base::kumikit::host::types,
        },
    });
}
