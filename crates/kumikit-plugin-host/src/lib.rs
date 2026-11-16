//! Kumikit plugin host: runs core plugins as WASM components. See
//! `docs/spec/04-plugin-host.md`.

pub mod engine;
pub mod lifecycle;
pub mod manager;
pub mod manifest;
pub mod package;
pub mod runtime;
