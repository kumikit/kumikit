//! The `kumikit` core: transports, the JSON-RPC server and the CLI around the kernel.
//! The binary in `main.rs` is a thin wrapper over this library so tests can drive it.

pub mod bundled;
pub mod cli;
pub mod lock;
pub mod logwriter;
pub mod messages;
pub mod paths;
pub mod plugins;
pub mod registry;
pub mod remote;
pub mod rpc;
pub mod serve;
pub mod signing;
pub mod theme;
pub mod transport;
