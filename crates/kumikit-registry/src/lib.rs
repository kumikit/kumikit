//! Kumikit plugin registries (`docs/spec/10-distribution.md#registry`,
//! [ADR 0018](../../../docs/decisions/0018-package-signing-and-registry.md)).
//!
//! Shared by the core (core plugins) and the desktop shell (UI plugins): each
//! side fetches what it installs. The registry's own CI uses `kumikit-registry-tool`, on
//! top of this crate.

pub mod client;
pub mod confusable;
pub mod fetch;
pub mod index;
pub mod release;
pub mod resolve;

pub use client::{Freshness, RegistryClient, RegistryError, RegistryList};
pub use fetch::{FetchError, Fetched, Fetcher, Limits};
pub use index::Index;
pub use release::{ReleaseAsset, ReleaseClient, ReleaseError, ReleaseManifest, UpdateCheck};
