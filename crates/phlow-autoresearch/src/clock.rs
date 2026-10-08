//! Clock abstraction so budget enforcement is deterministic under
//! test. Mirrors phlow-experiment's `Clock` / `ManualClock` precedent:
//! production code reads a monotonic system clock; tests drive a manual
//! one. All values are milliseconds since an arbitrary, per-clock epoch
//! — only differences are meaningful.

use std::cell::Cell;
use std::time::Instant;

/// A source of monotonic millisecond readings.
pub trait Clock {
    /// Milliseconds since this clock's epoch. Monotonic per clock.
    fn now_ms(&self) -> u64;

    /// Test-support hook: advance the clock by `ms`. The system clock
    /// cannot be advanced and ignores this; the manual clock honors it.
    /// Scripted evaluators use it to simulate slow evaluations without
    /// sleeping, so timeout tests stay deterministic.
    fn advance(&self, _ms: u64) {}
}

/// Monotonic clock backed by [`Instant`]; the production default.
#[derive(Debug)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    /// A clock whose epoch is now.
    #[must_use]
    pub fn new() -> Self {
        SystemClock {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// Manually advanced clock for deterministic tests.
#[derive(Debug, Default)]
pub struct ManualClock {
    now: Cell<u64>,
}

impl ManualClock {
    /// A manual clock starting at 0 ms.
    #[must_use]
    pub fn new() -> Self {
        ManualClock { now: Cell::new(0) }
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.get()
    }

    fn advance(&self, ms: u64) {
        self.now.set(self.now.get().saturating_add(ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_advances_monotonically() {
        let clock = ManualClock::new();
        assert_eq!(clock.now_ms(), 0);
        clock.advance(250);
        clock.advance(250);
        assert_eq!(clock.now_ms(), 500);
    }

    #[test]
    fn system_clock_ignores_advance() {
        let clock = SystemClock::new();
        clock.advance(60_000);
        assert!(clock.now_ms() < 60_000, "system clock must not jump");
    }
}
