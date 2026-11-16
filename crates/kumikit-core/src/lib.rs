//! The `kumikit` core: transports, the JSON-RPC server and the CLI around the kernel.
//! The binary in `main.rs` is a thin wrapper over this library so tests can drive it.

pub mod cli;
pub mod lock;
pub mod paths;
pub mod plugins;
pub mod rpc;
pub mod serve;
pub mod transport;
