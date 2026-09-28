//! Deterministic clocks. Production uses `SystemClock`; every gauntlet
//! driver uses `ManualClock` so timer behavior is exactly scriptable.

/// Seconds since the unix epoch.
pub trait Clock {
    fn now(&self) -> u64;
}

/// Wall clock. Used outside tests only.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Scriptable clock. Tests advance it explicitly; nothing moves on its
/// own, so timer races are impossible by construction.
#[derive(Clone, Debug)]
pub struct ManualClock {
    now: u64,
}

impl ManualClock {
    pub fn new(start: u64) -> Self {
        ManualClock { now: start }
    }

    pub fn advance(&mut self, secs: u64) {
        self.now = self.now.saturating_add(secs);
    }

    pub fn set(&mut self, t: u64) {
        self.now = t;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> u64 {
        self.now
    }
}
