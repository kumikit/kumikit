//! Guest-side SDK for Kumikit provider plugins: the world
//! `kumikit:provider/plugin@0.2.0` (`docs/spec/05-providers.md`), on top of the
//! `kumikit-plugin` crate, whose types it shares.
//!
//! Export a type implementing [`Plugin`] and [`Provider`] with [`export_provider!`].
//! Like `kumikit-plugin`, it builds natively too; the export macro expands to nothing
//! off `wasm32`.
#![deny(missing_docs)]

pub use kumikit_plugin::{
    CallContext, Child, CommandResult, Error, ErrorCode, ExitStatus, Plugin, SpawnRequest,
    WorkspaceRef, kv, log, process, secrets, settings,
};

/// Generated bindings for the provider world. Interfaces it shares with the base world
/// are the base world's types.
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod bindings {
    wit_bindgen::generate!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-plugin@0.2.0",
            "../../wit/kumikit-provider@0.2.0",
        ],
        world: "kumikit:provider/plugin",
        pub_export_macro: true,
        export_macro_name: "__export_provider_world",
        default_bindings_module: "kumikit_provider::bindings",
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
pub use bindings::__export_provider_world;
pub use bindings::exports::kumikit::provider::provider::Guest as Provider;
pub use bindings::kumikit::host::events::{self, Decision, Event};
pub use bindings::kumikit::provider::types::{
    McpServer, Model, ProviderRef, SessionConfig, UserInput,
};
#[doc(hidden)]
pub use kumikit_plugin::__forward_plugin_exports;

/// Exports a type implementing [`Plugin`] and [`Provider`] (both from this crate) as the
/// plugin's `kumikit:provider/plugin` world. Expands to nothing when not compiling for
/// `wasm32`.
#[macro_export]
macro_rules! export_provider {
    ($ty:ident) => {
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            struct __KumikitExports;
            $crate::__forward_plugin_exports!(__KumikitExports, $ty, $crate::bindings);
            impl $crate::Provider for __KumikitExports {
                fn list_models(
                    p: $crate::ProviderRef,
                ) -> Result<Vec<$crate::Model>, $crate::Error> {
                    <$ty as $crate::Provider>::list_models(p)
                }
                fn start_session(
                    p: $crate::ProviderRef,
                    c: $crate::SessionConfig,
                ) -> Result<String, $crate::Error> {
                    <$ty as $crate::Provider>::start_session(p, c)
                }
                fn send(s: String, i: $crate::UserInput) -> Result<(), $crate::Error> {
                    <$ty as $crate::Provider>::send(s, i)
                }
                fn end_session(s: String) {
                    <$ty as $crate::Provider>::end_session(s)
                }
            }
    $crate::__export_provider_world!(__KumikitExports with_types_in $crate::bindings);
        };
        #[cfg(not(target_arch = "wasm32"))]
        const _: () = {
            let _ = <$ty as $crate::Plugin>::activate;
            let _ = <$ty as $crate::Provider>::send;
        };
    };
}

/// Reads a child's stdout line by line.
#[derive(Debug)]
pub struct Lines<'a> {
    child: &'a Child,
    buf: Vec<u8>,
    eof: bool,
}

/// Longest line [`Lines`] accepts (8 MiB, the protocol's message limit).
pub const MAX_LINE: usize = 8 << 20;

impl<'a> Lines<'a> {
    /// Reads lines from `child`'s stdout.
    pub fn new(child: &'a Child) -> Self {
        Lines {
            child,
            buf: Vec::new(),
            eof: false,
        }
    }

    /// The next line without its line ending, or none at EOF. A last line without a
    /// newline is returned too. Blocks inside the host until data arrives.
    pub fn next_line(&mut self) -> Result<Option<String>, Error> {
        loop {
            if let Some(line) = take_line(&mut self.buf, self.eof) {
                return String::from_utf8(line)
                    .map(Some)
                    .map_err(|_| Error::new(ErrorCode::InvalidArgument, "line is not UTF-8"));
            }
            if self.eof {
                return Ok(None);
            }
            if self.buf.len() > MAX_LINE {
                return Err(Error::new(ErrorCode::Limit, "line longer than 8 MiB"));
            }
            match self.child.read_stdout(64 * 1024)? {
                Some(chunk) => self.buf.extend_from_slice(&chunk),
                None => self.eof = true,
            }
        }
    }
}

/// Removes and returns the first complete line from `buf` (without `\n` or `\r\n`); at
/// EOF, the rest of the buffer if it is not empty.
pub fn take_line(buf: &mut Vec<u8>, eof: bool) -> Option<Vec<u8>> {
    match buf.iter().position(|&b| b == b'\n') {
        Some(i) => {
            let mut line: Vec<u8> = buf.drain(..=i).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            Some(line)
        }
        None if eof && !buf.is_empty() => Some(std::mem::take(buf)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::take_line;

    #[test]
    fn splits_lines_and_keeps_partial_ones() {
        let mut buf = b"a\r\nb\nc".to_vec();
        assert_eq!(take_line(&mut buf, false).unwrap(), b"a");
        assert_eq!(take_line(&mut buf, false).unwrap(), b"b");
        assert_eq!(take_line(&mut buf, false), None);
        assert_eq!(take_line(&mut buf, true).unwrap(), b"c");
        assert_eq!(take_line(&mut buf, true), None);
    }
}
