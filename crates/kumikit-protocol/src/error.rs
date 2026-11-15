//! Error objects and the stable `data.code` strings clients react to.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

/// A stable error code, sent as `error.data.code`. See the table in
/// `docs/spec/02-protocol.md#errors`.
///
/// Deserializing an unknown code yields [`ErrorCode::Internal`], as the spec asks clients
/// to treat unknown codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The line is not JSON.
    ParseError,
    /// Not a JSON-RPC 2.0 request (including batches).
    InvalidRequest,
    /// No such method.
    MethodNotFound,
    /// Params do not match the method.
    InvalidParams,
    /// A referenced workspace, thread, turn or approval does not exist.
    NotFound,
    /// The caller or plugin lacks a permission.
    PermissionDenied,
    /// Known but not available on this core.
    Unsupported,
    /// The request contradicts current state.
    Conflict,
    /// A method was called before `core.hello`.
    HandshakeRequired,
    /// The client's protocol version cannot be served.
    ProtocolMismatch,
    /// A plugin or provider failed while handling the request.
    PluginFailed,
    /// A bug or an I/O failure in the core.
    Internal,
}

impl ErrorCode {
    /// Every code, in table order.
    pub const ALL: [ErrorCode; 12] = [
        ErrorCode::ParseError,
        ErrorCode::InvalidRequest,
        ErrorCode::MethodNotFound,
        ErrorCode::InvalidParams,
        ErrorCode::NotFound,
        ErrorCode::PermissionDenied,
        ErrorCode::Unsupported,
        ErrorCode::Conflict,
        ErrorCode::HandshakeRequired,
        ErrorCode::ProtocolMismatch,
        ErrorCode::PluginFailed,
        ErrorCode::Internal,
    ];

    /// The `data.code` string.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::ParseError => "parse_error",
            ErrorCode::InvalidRequest => "invalid_request",
            ErrorCode::MethodNotFound => "method_not_found",
            ErrorCode::InvalidParams => "invalid_params",
            ErrorCode::NotFound => "not_found",
            ErrorCode::PermissionDenied => "permission_denied",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::Conflict => "conflict",
            ErrorCode::HandshakeRequired => "handshake_required",
            ErrorCode::ProtocolMismatch => "protocol_mismatch",
            ErrorCode::PluginFailed => "plugin_failed",
            ErrorCode::Internal => "internal",
        }
    }

    /// The numeric JSON-RPC error code.
    pub fn rpc_code(self) -> i32 {
        match self {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            _ => -32000,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(ErrorCode::ALL
            .into_iter()
            .find(|c| c.as_str() == value)
            .unwrap_or(ErrorCode::Internal))
    }
}

/// Machine-readable error details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ErrorData {
    /// The stable code.
    pub code: ErrorCode,
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RpcError {
    /// Numeric JSON-RPC code, derived from `data.code`.
    pub code: i32,
    /// Human-readable message; not for matching.
    pub message: String,
    /// The stable code.
    pub data: ErrorData,
}

impl RpcError {
    /// An error with the given code and message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        RpcError {
            code: code.rpc_code(),
            message: message.into(),
            data: ErrorData { code },
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.data.code, self.message)
    }
}

impl std::error::Error for RpcError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_as_strings() {
        for code in ErrorCode::ALL {
            let json = serde_json::to_value(code).unwrap();
            assert_eq!(json, serde_json::Value::String(code.as_str().into()));
            assert_eq!(serde_json::from_value::<ErrorCode>(json).unwrap(), code);
        }
    }

    #[test]
    fn unknown_code_reads_as_internal() {
        let code: ErrorCode = serde_json::from_str("\"rate_limited\"").unwrap();
        assert_eq!(code, ErrorCode::Internal);
    }

    #[test]
    fn numeric_codes() {
        assert_eq!(ErrorCode::ParseError.rpc_code(), -32700);
        assert_eq!(ErrorCode::NotFound.rpc_code(), -32000);
    }
}
