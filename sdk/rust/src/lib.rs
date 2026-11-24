//! Guest-side SDK for Kumikit core plugins.
//!
//! A core plugin is a WASM component built for `wasm32-wasip2`. This crate generates the
//! bindings for Kumikit's WIT packages (`wit/` in the repository) and adds helpers on top.
//! See `docs/spec/04-plugin-host.md` and `docs/spec/05-providers.md`.
//!
//! This crate targets the base world `kumikit:plugin/plugin@0.2.0` (`log`, `kv`,
//! `process`, `secrets`, `settings`, and the exports); implement [`Plugin`] and export the
//! type with [`export_plugin!`]. Provider plugins use the
//! `kumikit-provider` crate (`sdk/rust-provider`), which builds on this one. (Each world
//! needs its own crate: wit-bindgen's export macros are crate-global and would collide.)
//!
//! Plugin crates also build natively: the export macros expand to nothing off `wasm32`,
//! and calling a host import natively panics. Keep logic that tests should cover free of
//! host calls (see `plugins/provider-acp` for the pattern).
#![deny(missing_docs)]

pub mod http;

/// Generated bindings for the base world.
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod bindings {
    wit_bindgen::generate!({
        path: ["../../wit/kumikit-host@0.2.0", "../../wit/kumikit-plugin@0.2.0"],
        world: "kumikit:plugin/plugin",
        pub_export_macro: true,
        export_macro_name: "__export_base_world",
        default_bindings_module: "kumikit_plugin::bindings",
        with: {
            "wasi:io/poll@0.2.12": wasip2::io::poll,
        },
        generate_all,
    });
}

/// Generated bindings for `kumikit:turn/client-steering@0.1.0` (ADR 0028).
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod turn_bindings {
    wit_bindgen::generate!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-plugin@0.2.0",
            "../../wit/kumikit-provider@0.2.0",
            "../../wit/kumikit-turn@0.1.0",
        ],
        world: "kumikit:turn/client-steering",
        with: {
            "kumikit:host/types@0.2.0": crate::bindings::kumikit::host::types,
        },
        generate_all,
    });
}

/// Steering running turns from a plugin (`docs/spec/03-kernel.md#client-interface`,
/// ADR 0028). Needs `threads.steer` in `permissions.client`.
pub mod turn {
    pub use crate::turn_bindings::kumikit::turn::client::{SteerResult, SteerStatus, steer};
}

#[doc(hidden)]
pub use bindings::__export_base_world;
pub use bindings::kumikit::host::process::{self, Child, ExitStatus, SpawnRequest};
pub use bindings::kumikit::host::types::{Error, ErrorCode};
pub use bindings::kumikit::host::{client, kv, log, secrets, settings, timers};
pub use bindings::kumikit::plugin::types::{CallContext, CommandResult, WorkspaceRef};
/// `wasi:io/poll`, shared with [`http`]: wait on several pollables with [`poll::poll`].
pub use wasip2::io::poll;

impl Error {
    /// An error with a code and a message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
        }
    }
}

/// What every plugin implements. Only `activate` and `deactivate` are required; the
/// others answer `not-found`, for plugins without methods, commands or schedules.
pub trait Plugin {
    /// Called once after each instance is created, before any other export.
    fn activate() -> Result<(), Error>;
    /// Called before an instance is dropped; keep it short.
    fn deactivate();
    /// A plugin method (`<plugin id>/<method>`); `params` and the result are JSON text.
    fn handle(method: String, params: String, ctx: CallContext) -> Result<String, Error> {
        let _ = (params, ctx);
        Err(Error::new(
            ErrorCode::NotFound,
            format!("no method {method}"),
        ))
    }
    /// A command from the manifest; `args` is JSON text.
    fn run_command(id: String, args: String, ctx: CallContext) -> Result<CommandResult, Error> {
        let _ = (args, ctx);
        Err(Error::new(ErrorCode::NotFound, format!("no command {id}")))
    }
    /// A schedule is due; `due` is RFC 3339 in UTC.
    fn fire(key: String, due: String) -> Result<(), Error> {
        let _ = due;
        Err(Error::new(
            ErrorCode::NotFound,
            format!("no schedule {key}"),
        ))
    }
}

/// Implements the world's export interfaces for `$exports` by forwarding to `$ty`'s
/// [`Plugin`] impl. `$b` is the bindings module of the world. Used by the export macros.
#[doc(hidden)]
#[macro_export]
macro_rules! __forward_plugin_exports {
    ($exports:ident, $ty:ty, $b:path) => {
        use $b as __kk_b;
        impl __kk_b::exports::kumikit::plugin::lifecycle::Guest for $exports {
            fn activate() -> Result<(), $crate::Error> {
                <$ty as $crate::Plugin>::activate()
            }
            fn deactivate() {
                <$ty as $crate::Plugin>::deactivate()
            }
        }
        impl __kk_b::exports::kumikit::plugin::rpc::Guest for $exports {
            fn handle(
                method: String,
                params: String,
                ctx: $crate::CallContext,
            ) -> Result<String, $crate::Error> {
                <$ty as $crate::Plugin>::handle(method, params, ctx)
            }
        }
        impl __kk_b::exports::kumikit::plugin::commands::Guest for $exports {
            fn run(
                id: String,
                args: String,
                ctx: $crate::CallContext,
            ) -> Result<$crate::CommandResult, $crate::Error> {
                <$ty as $crate::Plugin>::run_command(id, args, ctx)
            }
        }
        impl __kk_b::exports::kumikit::plugin::timers::Guest for $exports {
            fn fire(key: String, due: String) -> Result<(), $crate::Error> {
                <$ty as $crate::Plugin>::fire(key, due)
            }
        }
    };
}

/// Exports a type implementing [`Plugin`] as the plugin's `kumikit:plugin/plugin` world.
/// Expands to nothing when not compiling for `wasm32`.
#[macro_export]
macro_rules! export_plugin {
    ($ty:ident) => {
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            struct __KumikitExports;
            $crate::__forward_plugin_exports!(__KumikitExports, $ty, $crate::bindings);
    $crate::__export_base_world!(__KumikitExports with_types_in $crate::bindings);
        };
        // Natively, still check the trait bound and mark the type as used.
        #[cfg(not(target_arch = "wasm32"))]
        const _: () = {
            let _ = <$ty as $crate::Plugin>::activate;
        };
    };
}
