//! Kumikit plugin host: runs core plugins as WASM components. See
//! `docs/spec/04-plugin-host.md`.

pub mod engine;
pub mod lifecycle;
pub mod manager;
pub mod manifest;
/// `.kkp` packages, from the shared `kumikit-package` crate.
pub use kumikit_package as package;
pub mod runtime;
