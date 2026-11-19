//! Guest-side SDK for Kumikit core plugins.
//!
//! A core plugin is a WASM component built for `wasm32-wasip2`. This crate generates the
//! bindings for Kumikit's WIT packages (`wit/` in the repository) and adds helpers on top.
//! See `docs/spec/04-plugin-host.md` and `docs/spec/05-providers.md`.
//!
//! This crate targets the base world `kumikit:plugin/plugin@0.2.0` (`log`, `kv`,
//! `process`, `secrets`, `settings`, lifecycle); export a type with [`export_plugin!`]. Provider plugins use the
//! `kumikit-provider` crate (`sdk/rust-provider`), which builds on this one. (Each world
//! needs its own crate: wit-bindgen's export macros are crate-global and would collide.)
//!
//! Plugin crates also build natively: the export macros expand to nothing off `wasm32`,
//! and calling a host import natively panics. Keep logic that tests should cover free of
//! host calls (see `plugins/provider-acp` for the pattern).
#![deny(missing_docs)]

/// Generated bindings for the base world.
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod bindings {
    wit_bindgen::generate!({
        path: ["../../wit/kumikit-host@0.2.0", "../../wit/kumikit-plugin@0.2.0"],
        world: "kumikit:plugin/plugin",
        pub_export_macro: true,
        export_macro_name: "__export_base_world",
        default_bindings_module: "kumikit_plugin::bindings",
        generate_all,
    });
}

#[doc(hidden)]
pub use bindings::__export_base_world;
pub use bindings::exports::kumikit::plugin::lifecycle::Guest as Lifecycle;
pub use bindings::kumikit::host::process::{self, Child, ExitStatus, SpawnRequest};
pub use bindings::kumikit::host::types::{Error, ErrorCode};
pub use bindings::kumikit::host::{kv, log, secrets, settings};

impl Error {
    /// An error with a code and a message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
        }
    }
}

/// Exports a type implementing [`Lifecycle`] as the plugin's `kumikit:plugin/plugin`
/// world. Expands to nothing when not compiling for `wasm32`.
#[macro_export]
macro_rules! export_plugin {
    ($ty:ident) => {
        #[cfg(target_arch = "wasm32")]
        $crate::__export_base_world!($ty);
        // Natively, still check the trait bound and mark the type as used.
        #[cfg(not(target_arch = "wasm32"))]
        const _: () = {
            let _ = <$ty as $crate::Lifecycle>::activate;
        };
    };
}
