//! Kumikit client–core protocol: JSON-RPC envelopes, the method table, domain types and
//! thread events. This crate has no I/O; see `docs/spec/02-protocol.md`.
#![deny(missing_docs)]

pub mod error;
pub mod event;
pub mod jsonrpc;
pub mod local;
pub mod lockfile;
pub mod methods;
pub mod model;
pub mod remote;
pub mod topic;
pub mod version;

pub use error::{ErrorCode, RpcError};
pub use event::{EventNotification, ThreadEvent, TopicEvent, WorkspaceEvent};
pub use methods::{Method, NotificationMethod};
pub use topic::Topic;
pub use version::{APP_VERSION, PROTOCOL_VERSION, ProtocolVersion};
