//! The Kumikit desktop shell ([06-client](../../../docs/spec/06-client.md)): serving the
//! UI, finding or starting the local core, the connection bridge and the client plugin
//! store. The logic lives here so it is tested without a window; `main.rs` wires it into
//! Tauri.

pub mod assets;
pub mod bridge;
pub mod local;
pub mod store;
#[cfg(feature = "test-hooks")]
pub mod test_hooks;
pub mod transport;
