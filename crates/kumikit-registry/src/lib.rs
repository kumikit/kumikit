//! Kumikit plugin registries (`docs/spec/10-distribution.md#registry`,
//! [ADR 0018](../../../docs/decisions/0018-package-signing-and-registry.md)).
//!
//! Shared by the core (core plugins) and the desktop shell (UI plugins and themes): each
//! side fetches what it installs.

pub mod fetch;

pub use fetch::{FetchError, Fetched, Fetcher, Limits};
