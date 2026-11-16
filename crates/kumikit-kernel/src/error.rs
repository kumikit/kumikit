//! Kernel errors and their protocol codes.

use kumikit_protocol::ErrorCode;

use crate::store::StoreError;

/// An error from a kernel operation. Each variant maps to one protocol error code.
#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    /// A referenced object does not exist.
    #[error("{0}")]
    NotFound(String),
    /// Params are malformed or out of range.
    #[error("{0}")]
    InvalidParams(String),
    /// The request contradicts current state.
    #[error("{0}")]
    Conflict(String),
    /// Known but not available.
    #[error("{0}")]
    Unsupported(String),
    /// A provider failed.
    #[error("{0}")]
    PluginFailed(String),
    /// A size or count limit was exceeded (reported as `invalid_params` on the wire).
    #[error("{0}")]
    Limit(String),
    /// Reading or writing a file failed.
    #[error("{0}")]
    Io(String),
    /// Storage failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl KernelError {
    /// The protocol error code for this error.
    pub fn code(&self) -> ErrorCode {
        match self {
            KernelError::NotFound(_) => ErrorCode::NotFound,
            KernelError::InvalidParams(_) => ErrorCode::InvalidParams,
            KernelError::Conflict(_) => ErrorCode::Conflict,
            KernelError::Unsupported(_) => ErrorCode::Unsupported,
            KernelError::PluginFailed(_) => ErrorCode::PluginFailed,
            KernelError::Limit(_) => ErrorCode::InvalidParams,
            KernelError::Io(_) => ErrorCode::Internal,
            KernelError::Store(_) => ErrorCode::Internal,
        }
    }
}

impl From<rusqlite::Error> for KernelError {
    fn from(e: rusqlite::Error) -> Self {
        KernelError::Store(StoreError::Sqlite(e))
    }
}

impl From<serde_json::Error> for KernelError {
    fn from(e: serde_json::Error) -> Self {
        KernelError::Store(StoreError::Json(e))
    }
}

/// Result alias for kernel calls.
pub type KernelResult<T> = Result<T, KernelError>;
