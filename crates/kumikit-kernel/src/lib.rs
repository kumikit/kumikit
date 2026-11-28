//! Kumikit kernel: domain model, event log, storage and the internal provider trait.
//! See `docs/spec/03-kernel.md`.

pub mod clock;
pub mod defer;
pub mod devices;
pub mod error;
pub mod fold;
pub mod gateway;
mod git;
pub mod grants;
pub mod handoff;
mod kernel;
pub mod plugins;
pub mod process;
pub mod provider;
pub mod redact;
pub mod repo;
pub mod schedule;
pub mod secrets;
pub mod store;
mod turns;
pub mod usage;

pub use error::{KernelError, KernelResult};
pub use kernel::{CORE_RESTARTED, Kernel, KernelConfig, ThreadSubscription};
