//! Guest-side SDK for Kumikit tool plugins: the world `kumikit:tool/plugin@0.1.0`
//! (`docs/spec/03-kernel.md#tool-gateway`), on top of the `kumikit-plugin` crate, whose
//! types it shares.
//!
//! List the tools in the manifest (`[[contributes.tools]]`), implement [`Plugin`] and
//! [`Tool`], and export the type with [`export_tool!`]. Like `kumikit-plugin`, it builds
//! natively too; the export macro expands to nothing off `wasm32`.
#![deny(missing_docs)]

pub use kumikit_plugin::http;
pub use kumikit_plugin::{
    CallContext, Child, CommandResult, Error, ErrorCode, ExitStatus, Plugin, SpawnRequest,
    WorkspaceRef, client, kv, log, process, secrets, settings, threads, turn,
};

/// Generated bindings for the tool world. Interfaces it shares with the base world are the
/// base world's types.
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod bindings {
    wit_bindgen::generate!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-plugin@0.2.0",
            "../../wit/kumikit-tool@0.1.0",
        ],
        world: "kumikit:tool/plugin",
        pub_export_macro: true,
        export_macro_name: "__export_tool_world",
        default_bindings_module: "kumikit_tool::bindings",
        with: {
            "kumikit:host/types@0.2.0": kumikit_plugin::bindings::kumikit::host::types,
            "kumikit:host/log@0.2.0": kumikit_plugin::bindings::kumikit::host::log,
            "kumikit:host/kv@0.2.0": kumikit_plugin::bindings::kumikit::host::kv,
            "kumikit:host/process@0.2.0": kumikit_plugin::bindings::kumikit::host::process,
            "kumikit:host/secrets@0.2.0": kumikit_plugin::bindings::kumikit::host::secrets,
            "kumikit:host/settings@0.2.0": kumikit_plugin::bindings::kumikit::host::settings,
            "kumikit:plugin/types@0.2.0": kumikit_plugin::bindings::kumikit::plugin::types,
        },
        generate_all,
    });
}

#[doc(hidden)]
pub use bindings::__export_tool_world;
pub use bindings::exports::kumikit::tool::tool::Guest as Tool;
pub use bindings::exports::kumikit::tool::tool::{ToolCall, ToolResult};
#[doc(hidden)]
pub use kumikit_plugin::__forward_plugin_exports;

impl ToolResult {
    /// A successful result with text for the agent.
    pub fn text(text: impl Into<String>) -> Self {
        ToolResult {
            text: text.into(),
            structured: None,
            is_error: false,
        }
    }

    /// A result that says the tool ran but failed.
    pub fn failed(text: impl Into<String>) -> Self {
        ToolResult {
            text: text.into(),
            structured: None,
            is_error: true,
        }
    }
}

/// Exports a type implementing [`Plugin`] and [`Tool`] as the plugin's
/// `kumikit:tool/plugin` world. Expands to nothing when not compiling for `wasm32`.
#[macro_export]
macro_rules! export_tool {
    ($ty:ident) => {
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            struct __KumikitExports;
            $crate::__forward_plugin_exports!(__KumikitExports, $ty, $crate::bindings);
            impl $crate::Tool for __KumikitExports {
                fn call(c: $crate::ToolCall) -> Result<$crate::ToolResult, $crate::Error> {
                    <$ty as $crate::Tool>::call(c)
                }
            }
    $crate::__export_tool_world!(__KumikitExports with_types_in $crate::bindings);
        };
        #[cfg(not(target_arch = "wasm32"))]
        const _: () = {
            let _ = <$ty as $crate::Plugin>::activate;
            let _ = <$ty as $crate::Tool>::call;
        };
    };
}
