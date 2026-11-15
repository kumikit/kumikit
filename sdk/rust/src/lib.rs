//! Guest-side SDK for Kumikit core plugins.
//!
//! A core plugin is a WASM component built for `wasm32-wasip2`. This crate generates the
//! bindings for Kumikit's WIT packages (`wit/` in the repository) and adds helpers on top.
//! See `docs/spec/04-plugin-host.md`.
#![deny(missing_docs)]

/// Generated bindings for the `kumikit:plugin/plugin` world.
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod bindings {
    wit_bindgen::generate!({
        path: ["../../wit/kumikit-host@0.1.0", "../../wit/kumikit-plugin@0.1.0"],
        world: "kumikit:plugin/plugin",
        pub_export_macro: true,
        export_macro_name: "__export_plugin_world",
        default_bindings_module: "kumikit_plugin::bindings",
        generate_all,
    });
}

#[doc(hidden)]
pub use bindings::__export_plugin_world;

/// Exports a type implementing [`Lifecycle`] as the plugin's `kumikit:plugin/plugin`
/// world.
///
/// Expands to nothing when not compiling for `wasm32`, so a plugin crate also builds
/// natively and its logic can be unit-tested with plain `cargo test`.
#[macro_export]
macro_rules! export_plugin {
    ($ty:ident) => {
        #[cfg(target_arch = "wasm32")]
        $crate::__export_plugin_world!($ty);
        // Natively, still check the trait bound and mark the type as used.
        #[cfg(not(target_arch = "wasm32"))]
        const _: () = {
            let _ = <$ty as $crate::Lifecycle>::activate;
        };
    };
}
pub use bindings::exports::kumikit::plugin::lifecycle::Guest as Lifecycle;
pub use bindings::kumikit::host::log;
pub use bindings::kumikit::host::types::{Error, ErrorCode};
