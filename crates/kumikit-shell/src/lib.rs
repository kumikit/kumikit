//! The Kumikit desktop shell ([06-client](../../../docs/spec/06-client.md)): serving the
//! UI, finding or starting the local core, the connection bridge, remote profiles and
//! the client plugin store. The logic lives here so it is tested without a window; `main.rs` wires it into
//! Tauri.

/// Serving UI files; shared with the core (`kumikit-assets`).
pub use kumikit_assets as assets;
pub mod bridge;
pub mod client_registry;
pub mod connections;
pub mod local;
pub mod profiles;
pub mod remote;
pub mod secrets;
pub mod ssh;
pub mod store;
#[cfg(feature = "test-hooks")]
pub mod test_hooks;
pub mod transport;
