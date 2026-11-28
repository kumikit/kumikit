//! The core process's own resource use for `core.stats` (`docs/spec/02-protocol.md`):
//! resident memory and uptime. The kernel adds the database size, turns and processes.

use std::sync::OnceLock;
use std::time::Instant;

static STARTED: OnceLock<Instant> = OnceLock::new();

/// Records when the core started; `serve` calls it first. Later calls change nothing.
pub fn mark_start() {
    STARTED.get_or_init(Instant::now);
}

/// Seconds since [`mark_start`] (or since the first call, if it was never called).
pub fn uptime_seconds() -> u64 {
    STARTED.get_or_init(Instant::now).elapsed().as_secs()
}

/// Resident memory of this process in bytes, where the OS reports it.
pub fn memory_bytes() -> Option<u64> {
    memory_stats::memory_stats().map(|m| m.physical_mem as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_is_reported_on_supported_platforms() {
        // Linux, macOS and Windows all report it; a running test uses some.
        assert!(memory_bytes().is_some_and(|b| b > 0));
        mark_start();
        assert!(uptime_seconds() < 3600);
    }
}
