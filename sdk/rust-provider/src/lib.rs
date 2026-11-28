//! Guest-side SDK for Kumikit provider plugins: the world
//! `kumikit:provider/plugin@0.2.0` (`docs/spec/05-providers.md`), on top of the
//! `kumikit-plugin` crate, whose types it shares.
//!
//! Export a type implementing [`Plugin`] and [`Provider`] with [`export_provider!`].
//! Like `kumikit-plugin`, it builds natively too; the export macro expands to nothing
//! off `wasm32`.
#![deny(missing_docs)]

pub use kumikit_plugin::http;
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

/// Generated bindings for `kumikit:turn/steering@0.1.0`, the steering import (ADR 0028).
/// A provider that uses [`steering`] must list `kumikit:turn/provider@0.1.0` in its
/// manifest's `runtime.worlds`, next to `kumikit:provider/plugin@0.2.0`.
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod turn_bindings {
    wit_bindgen::generate!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-plugin@0.2.0",
            "../../wit/kumikit-provider@0.2.0",
            "../../wit/kumikit-turn@0.1.0",
        ],
        world: "kumikit:turn/steering",
        with: {
            "kumikit:host/types@0.2.0": kumikit_plugin::bindings::kumikit::host::types,
            "kumikit:host/process@0.2.0": kumikit_plugin::bindings::kumikit::host::process,
            "kumikit:provider/types@0.2.0": crate::bindings::kumikit::provider::types,
        },
        generate_all,
    });
}

/// Generated bindings for `kumikit:session/provider-events@0.1.0` (slash commands and
/// usage limits, M7 steps 7-8).
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod session_bindings {
    wit_bindgen::generate!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-session@0.1.0",
        ],
        world: "kumikit:session/provider-events",
        with: {
            "kumikit:host/types@0.2.0": kumikit_plugin::bindings::kumikit::host::types,
        },
        generate_all,
    });
}

/// Generated bindings for `kumikit:models/provider-models@0.1.0` (model settings,
/// ADR 0033).
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod models_bindings {
    wit_bindgen::generate!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-models@0.1.0",
        ],
        world: "kumikit:models/provider-models",
        with: {
            "kumikit:host/types@0.2.0": kumikit_plugin::bindings::kumikit::host::types,
        },
        generate_all,
    });
}

/// Model settings such as effort or speed (`docs/spec/05-providers.md#model-settings`).
///
/// Call [`set_options`] from [`Provider::list_models`] for each model that has settings.
/// The values the user picks reach [`Provider::start_session`] as `option.<id>` entries of
/// [`SessionConfig::options`]; [`chosen`] reads one.
pub mod models {
    pub use crate::models_bindings::kumikit::models::options::{
        ModelOption, OptionValue, set_options,
    };

    /// The prefix of a model setting's key in `SessionConfig::options`.
    pub const OPTION_PREFIX: &str = "option.";

    /// The value chosen for setting `id` in a session's options, if any.
    pub fn chosen<'a>(options: &'a [(String, String)], id: &str) -> Option<&'a str> {
        options
            .iter()
            .find(|(k, _)| k.strip_prefix(OPTION_PREFIX) == Some(id))
            .map(|(_, v)| v.as_str())
    }

    #[cfg(test)]
    mod tests {
        use super::chosen;

        #[test]
        fn chosen_reads_prefixed_keys_only() {
            let opts = vec![
                ("permission-mode".to_owned(), "plan".to_owned()),
                ("option.effort".to_owned(), "high".to_owned()),
            ];
            assert_eq!(chosen(&opts, "effort"), Some("high"));
            assert_eq!(chosen(&opts, "permission-mode"), None);
            assert_eq!(chosen(&opts, "speed"), None);
        }
    }
}

/// Generated bindings for `kumikit:usage/reporter@0.1.0`: plan limits and usage by model
/// a provider reports during a turn (ADR 0035).
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod usage_bindings {
    wit_bindgen::generate!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-plugin@0.2.0",
            "../../wit/kumikit-provider@0.2.0",
            "../../wit/kumikit-usage@0.1.0",
        ],
        world: "kumikit:usage/reporter",
        with: {
            "kumikit:host/types@0.2.0": kumikit_plugin::bindings::kumikit::host::types,
        },
        generate_all,
    });
}

/// Generated bindings for `kumikit:usage/reader@0.1.0`, the on-demand plan limits reader
/// (ADR 0035). Export it with [`export_limits_reader!`].
#[allow(missing_docs, clippy::all, unsafe_code)]
pub mod usage_reader_bindings {
    wit_bindgen::generate!({
        path: [
            "../../wit/kumikit-host@0.2.0",
            "../../wit/kumikit-plugin@0.2.0",
            "../../wit/kumikit-provider@0.2.0",
            "../../wit/kumikit-usage@0.1.0",
        ],
        world: "kumikit:usage/reader",
        pub_export_macro: true,
        export_macro_name: "__export_usage_reader_world",
        default_bindings_module: "kumikit_provider::usage_reader_bindings",
        with: {
            "kumikit:host/types@0.2.0": kumikit_plugin::bindings::kumikit::host::types,
            "kumikit:provider/types@0.2.0": crate::bindings::kumikit::provider::types,
            "kumikit:usage/types@0.1.0": crate::usage_bindings::kumikit::usage::types,
        },
        generate_all,
    });
}

/// Plan limits and token usage by model (`docs/spec/05-providers.md#usage-and-limits`,
/// ADR 0035).
///
/// - During [`Provider::send`], [`emit_limits`] reports a snapshot of the account's plan
///   limits the agent showed, and [`emit_turn_usage`] reports the turn's usage by model
///   (use it instead of the plain `usage` event).
/// - To read limits on demand, implement [`LimitsReader`], export it with
///   [`export_limits_reader!`](crate::export_limits_reader) and list
///   `kumikit:usage/reader@0.1.0` in the manifest's `runtime.worlds`.
pub mod usage {
    pub use crate::usage_bindings::kumikit::usage::report::{emit_limits, emit_turn_usage};
    pub use crate::usage_bindings::kumikit::usage::types::{LimitWindow, Limits, ModelUsage};
    pub use crate::usage_reader_bindings::exports::kumikit::usage::probe::Guest as LimitsReader;
}

#[doc(hidden)]
pub use usage_reader_bindings::__export_usage_reader_world;

/// Exports a type implementing [`usage::LimitsReader`] as the plugin's
/// `kumikit:usage/reader` world, next to its main world. Expands to nothing when not
/// compiling for `wasm32`.
#[macro_export]
macro_rules! export_limits_reader {
    ($ty:ident) => {
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            struct __KumikitLimitsReader;
            impl $crate::usage::LimitsReader for __KumikitLimitsReader {
                fn read_limits(
                    p: $crate::ProviderRef,
                ) -> Result<Option<$crate::usage::Limits>, $crate::Error> {
                    <$ty as $crate::usage::LimitsReader>::read_limits(p)
                }
            }
    $crate::__export_usage_reader_world!(__KumikitLimitsReader with_types_in $crate::usage_reader_bindings);
        };
        #[cfg(not(target_arch = "wasm32"))]
        const _: () = {
            let _ = <$ty as $crate::usage::LimitsReader>::read_limits;
        };
    };
}

/// Additional session events (`commands.available` and `limit.reached`).
pub mod session {
    pub use crate::session_bindings::kumikit::session::events::{
        CommandInfo, emit_commands, emit_limit,
    };
}

/// Steering: messages a user sends while a turn runs (`docs/spec/05-providers.md#steering`).
///
/// Only meaningful inside [`Provider::send`]. Report the `steer` capability on models
/// whose agent can take input mid-turn, and list `kumikit:turn/provider@0.1.0` in the
/// manifest. A message is recorded as `input.steered` when it is taken, so pass every
/// taken message to the agent.
pub mod steering {
    pub use crate::turn_bindings::kumikit::turn::input::{Read, read_stdout, take};
    use crate::{Child, Error, ErrorCode, MAX_LINE, UserInput, take_line};

    /// What [`SteerableLines::next_item`] returned.
    #[derive(Debug)]
    pub enum Next {
        /// A line of the child's stdout, without its line ending.
        Line(String),
        /// A steering message, already taken: pass it to the agent.
        Input(UserInput),
        /// The child's stdout closed.
        Eof,
    }

    /// Reads a child's stdout line by line, like [`crate::Lines`], and also returns
    /// steering messages as they arrive.
    #[derive(Debug)]
    pub struct SteerableLines<'a> {
        child: &'a Child,
        buf: Vec<u8>,
        eof: bool,
    }

    impl<'a> SteerableLines<'a> {
        /// Reads lines from `child`'s stdout.
        pub fn new(child: &'a Child) -> Self {
            SteerableLines {
                child,
                buf: Vec::new(),
                eof: false,
            }
        }

        /// The next complete line, a steering message, or EOF. Blocks inside the host
        /// until one of them arrives.
        pub fn next_item(&mut self) -> Result<Next, Error> {
            loop {
                if let Some(line) = take_line(&mut self.buf, self.eof) {
                    return String::from_utf8(line)
                        .map(Next::Line)
                        .map_err(|_| Error::new(ErrorCode::InvalidArgument, "line is not UTF-8"));
                }
                if self.eof {
                    return Ok(Next::Eof);
                }
                if self.buf.len() > MAX_LINE {
                    return Err(Error::new(ErrorCode::Limit, "line longer than 8 MiB"));
                }
                match read_stdout(self.child, 64 * 1024)? {
                    Read::Stdout(chunk) => self.buf.extend_from_slice(&chunk),
                    Read::Eof => self.eof = true,
                    Read::Input(input) => return Ok(Next::Input(input)),
                }
            }
        }
    }
}

/// Renders a conversation handoff context into the shared prompt preamble (ADR 0029).
pub mod handoff {
    /// Speaker of a handoff message.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Role {
        /// The user.
        User,
        /// The agent.
        Assistant,
    }

    /// One plain-text user or assistant message carried across a handoff.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Message {
        /// Who wrote the message.
        pub role: Role,
        /// Plain text of the message.
        pub text: String,
    }

    /// Conversation context passed when another provider takes over a thread.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Context {
        /// Provider that ran the earlier conversation.
        pub from_provider: String,
        /// Model that ran the earlier conversation.
        pub from_model: String,
        /// Bounded transcript messages, oldest first.
        pub messages: Vec<Message>,
        /// Workspace-relative paths of files changed earlier.
        pub files_changed: Vec<String>,
        /// Whether earlier messages were omitted to fit the handoff limit.
        pub truncated: bool,
        /// Optional summary note from the outgoing provider.
        pub summary: Option<String>,
        /// Whether a requested summary could not be generated.
        pub summary_failed: bool,
    }

    /// Renders `ctx` as a prompt preamble matching the kernel's handoff format.
    pub fn render(ctx: &Context) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "[Conversation handoff from {} ({})]\n",
            ctx.from_provider, ctx.from_model
        ));
        if ctx.summary_failed {
            out.push_str(
                "Note: an automatic handoff summary could not be generated; the conversation transcript is included below.\n",
            );
        }
        if let Some(summary) = &ctx.summary {
            out.push_str("\n## Handoff summary\n");
            out.push_str(summary.trim());
            out.push('\n');
        }
        if !ctx.files_changed.is_empty() {
            out.push_str("\n## Files changed earlier\n");
            for path in &ctx.files_changed {
                out.push_str("- ");
                out.push_str(path);
                out.push('\n');
            }
        }
        if ctx.truncated {
            out.push_str(
                "\n(Earlier messages were omitted to stay within the 200-message / 64 KiB handoff limit.)\n",
            );
        }
        if !ctx.messages.is_empty() {
            out.push_str("\n## Earlier conversation\n");
            for msg in &ctx.messages {
                let label = match msg.role {
                    Role::User => "User",
                    Role::Assistant => "Assistant",
                };
                out.push_str("\n### ");
                out.push_str(label);
                out.push('\n');
                out.push_str(&msg.text);
                if !msg.text.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        out.push_str("\n[End of handoff — continue with the user's next message below]");
        out
    }
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
