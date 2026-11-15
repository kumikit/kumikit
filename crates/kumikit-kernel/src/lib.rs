//! Kumikit kernel: domain model, event log, storage and the internal provider trait.
//! See `docs/spec/03-kernel.md`.

pub mod clock;
pub mod error;
pub mod fold;
mod kernel;
pub mod repo;
pub mod store;

pub use error::{KernelError, KernelResult};
pub use kernel::{CORE_RESTARTED, Kernel, KernelConfig, ThreadSubscription};
