//! Time and id sources, injected so tests are deterministic.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use kumikit_protocol::model::Timestamp;
use time::OffsetDateTime;
use time::macros::format_description;

/// Source of event timestamps.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// The current time as RFC 3339 UTC with milliseconds.
    fn now(&self) -> Timestamp;
}

/// Source of new ids.
pub trait IdGen: Send + Sync + std::fmt::Debug {
    /// A new unique id.
    fn next(&self) -> String;
}

/// The system clock.
#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        format_time(OffsetDateTime::now_utc())
    }
}

/// Formats as `2026-11-15T10:00:00.000Z`.
pub fn format_time(t: OffsetDateTime) -> Timestamp {
    let fmt =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");
    let t = t.to_offset(time::UtcOffset::UTC);
    // Formatting a valid OffsetDateTime with a fixed, all-numeric description cannot
    // fail; fall back to the Unix epoch rather than panic if it ever does.
    Timestamp(
        t.format(&fmt)
            .unwrap_or_else(|_| "1970-01-01T00:00:00.000Z".to_owned()),
    )
}

/// ULIDs, monotonic within a millisecond.
#[derive(Debug, Default)]
pub struct UlidGen {
    generator: Mutex<ulid::Generator>,
}

impl IdGen for UlidGen {
    fn next(&self) -> String {
        let mut g = self.generator.lock().unwrap_or_else(|p| p.into_inner());
        // `generate` fails only when the random part overflows within one millisecond
        // (2^80 ids); fall back to a fresh, non-monotonic ULID then.
        g.generate()
            .unwrap_or_else(|_| ulid::Ulid::generate())
            .to_string()
    }
}

/// A clock that advances one second per call, from 2026-11-15T10:00:00Z. For tests.
#[derive(Debug, Default)]
pub struct StepClock {
    ticks: AtomicU64,
}

impl Clock for StepClock {
    fn now(&self) -> Timestamp {
        let n = self.ticks.fetch_add(1, Ordering::Relaxed);
        let secs = i64::try_from(n).unwrap_or(i64::MAX);
        let base = time::macros::datetime!(2026-11-15 10:00:00 UTC);
        format_time(base.saturating_add(time::Duration::seconds(secs)))
    }
}

/// Ids `<prefix>0001`, `<prefix>0002`, …, zero-padded to 26 characters like a ULID.
/// For tests.
#[derive(Debug)]
pub struct SequentialIds {
    prefix: String,
    next: AtomicU64,
}

impl SequentialIds {
    /// A generator with the given prefix (keep it short and uppercase).
    pub fn new(prefix: &str) -> Self {
        SequentialIds {
            prefix: prefix.to_owned(),
            next: AtomicU64::new(1),
        }
    }
}

impl IdGen for SequentialIds {
    fn next(&self) -> String {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let width = 26usize.saturating_sub(self.prefix.len());
        format!("{}{n:0width$}", self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_with_millis() {
        let t = time::macros::datetime!(2026-11-15 10:00:00.123456 UTC);
        assert_eq!(format_time(t).0, "2026-11-15T10:00:00.123Z");
    }

    #[test]
    fn step_clock_advances() {
        let c = StepClock::default();
        assert_eq!(c.now().0, "2026-11-15T10:00:00.000Z");
        assert_eq!(c.now().0, "2026-11-15T10:00:01.000Z");
    }

    #[test]
    fn ids_are_unique_and_sized() {
        let g = UlidGen::default();
        let (a, b) = (g.next(), g.next());
        assert_ne!(a, b);
        assert_eq!(a.len(), 26);
        let s = SequentialIds::new("T");
        assert_eq!(s.next(), "T0000000000000000000000001");
    }
}
