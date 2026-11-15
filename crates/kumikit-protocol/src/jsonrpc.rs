//! JSON-RPC 2.0 envelopes as used on every Kumikit transport.
//!
//! Params and results are carried as [`serde_json::Value`] here; the typed shapes live in
//! [`crate::methods`]. Batches are not supported (see `docs/spec/02-protocol.md`).

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use ts_rs::TS;

use crate::error::RpcError;

/// The `"jsonrpc": "2.0"` marker. Deserializing any other value fails.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, TS)]
#[ts(type = "\"2.0\"")]
pub struct Version;

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value == "2.0" {
            Ok(Version)
        } else {
            Err(serde::de::Error::custom(format!(
                "unsupported jsonrpc version {value:?}"
            )))
        }
    }
}

/// A request id: a number or a string, chosen by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum RequestId {
    /// A numeric id.
    Number(i64),
    /// A string id.
    String(String),
}

/// A call from a client to the core. Without an `id` it is a notification and gets no
/// response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(optional_fields)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// Absent for notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    /// Method name, for example `thread.create`.
    pub method: String,
    /// Method params; absent means `{}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A notification from the core, for example `events.event`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Notification {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// Notification name.
    pub method: String,
    /// Notification params.
    pub params: Value,
}

/// A successful response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct SuccessResponse {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// The id of the request this answers.
    pub id: RequestId,
    /// The method's result.
    pub result: Value,
}

/// An error response. `id` is `null` when the request could not be read far enough to
/// find its id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ErrorResponse {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// The id of the request this answers, or `null`.
    pub id: Option<RequestId>,
    /// What went wrong.
    pub error: RpcError,
}

/// A response to a [`Request`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum Response {
    /// The call succeeded.
    Success(SuccessResponse),
    /// The call failed.
    Error(ErrorResponse),
}

/// Anything the core sends to a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum ServerMessage {
    /// A response to a request.
    Response(Response),
    /// A notification.
    Notification(Notification),
}

impl Response {
    /// A success response for `id`.
    pub fn success(id: RequestId, result: Value) -> Self {
        Response::Success(SuccessResponse {
            jsonrpc: Version,
            id,
            result,
        })
    }

    /// An error response for `id` (or `null`).
    pub fn error(id: Option<RequestId>, error: RpcError) -> Self {
        Response::Error(ErrorResponse {
            jsonrpc: Version,
            id,
            error,
        })
    }
}

impl Notification {
    /// A notification with the given method and params.
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Notification {
            jsonrpc: Version,
            method: method.into(),
            params,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use serde_json::json;

    #[test]
    fn request_without_params_or_id() {
        let r: Request = serde_json::from_value(json!({"jsonrpc":"2.0","method":"x"})).unwrap();
        assert_eq!(r.id, None);
        assert_eq!(r.params, None);
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({"jsonrpc":"2.0","method":"x"})
        );
    }

    #[test]
    fn rejects_other_versions() {
        let r = serde_json::from_value::<Request>(json!({"jsonrpc":"1.0","id":1,"method":"x"}));
        assert!(r.is_err());
    }

    #[test]
    fn ids_are_numbers_or_strings() {
        let r: Request =
            serde_json::from_value(json!({"jsonrpc":"2.0","id":"a","method":"x"})).unwrap();
        assert_eq!(r.id, Some(RequestId::String("a".into())));
        let r: Request =
            serde_json::from_value(json!({"jsonrpc":"2.0","id":7,"method":"x"})).unwrap();
        assert_eq!(r.id, Some(RequestId::Number(7)));
    }

    #[test]
    fn error_response_with_null_id() {
        let resp = Response::error(None, RpcError::new(ErrorCode::ParseError, "bad json"));
        assert_eq!(
            serde_json::to_value(&resp).unwrap(),
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"bad json","data":{"code":"parse_error"}}})
        );
    }

    #[test]
    fn server_message_distinguishes_kinds() {
        let n: ServerMessage =
            serde_json::from_value(json!({"jsonrpc":"2.0","method":"events.event","params":{}}))
                .unwrap();
        assert!(matches!(n, ServerMessage::Notification(_)));
        let s: ServerMessage =
            serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"result":{}})).unwrap();
        assert!(matches!(s, ServerMessage::Response(Response::Success(_))));
    }
}
