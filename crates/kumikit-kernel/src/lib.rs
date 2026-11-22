//! Kumikit kernel: domain model, event log, storage and the internal provider trait.
//! See `docs/spec/03-kernel.md`.

pub mod clock;
pub mod devices;
pub mod error;
pub mod fold;
pub mod gateway;
mod git;
pub mod grants;
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

pub use error::{KernelError, KernelResult};
pub use kernel::{CORE_RESTARTED, Kernel, KernelConfig, ThreadSubscription};
