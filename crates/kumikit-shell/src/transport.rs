//! The client side of the local transport (`docs/spec/02-protocol.md#transports`).

use std::io;

/// A connection to a local core.
#[cfg(unix)]
pub type Stream = tokio::net::UnixStream;
/// A connection to a local core.
#[cfg(windows)]
pub type Stream = tokio::net::windows::named_pipe::NamedPipeClient;

/// Connects to a core's socket.
#[cfg(unix)]
pub async fn connect(address: &str) -> io::Result<Stream> {
    tokio::net::UnixStream::connect(address).await
}

/// Connects to a core's pipe, retrying while all its instances are busy.
#[cfg(windows)]
pub async fn connect(address: &str) -> io::Result<Stream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    /// `ERROR_PIPE_BUSY` from WinError.h.
    const ERROR_PIPE_BUSY: i32 = 231;
    loop {
        match ClientOptions::new().open(address) {
            Ok(client) => return Ok(client),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                // The core creates the next instance right after accepting one.
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            Err(e) => return Err(e),
        }
    }
}
