//! Local transports (`docs/spec/02-protocol.md#local-transport-details`): a Unix domain
//! socket on Linux and macOS, a named pipe on Windows. Both carry newline-delimited
//! JSON-RPC, served by [`crate::rpc`].

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{Listener, connect, current_uid, ensure_private_dir, peer_allowed};

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;
#[cfg(windows)]
pub use windows::{Listener, connect};

/// The current user id (Unix) or 0 (Windows, where it is not used).
#[cfg(windows)]
pub fn current_uid() -> u32 {
    0
}
