//! The cycle scheduler: bounded concurrency, testing windows, and
//! platform rate limits. All three bounds are structural — `tick` simply
//! does not emit launches that violate them.

use crate::bounty::clock::Clock;
use crate::bounty::types::{Program, Run, RunState, Target};

/// What the scheduler decided on one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchedAction {
    /// Launch a probe now.
    Launch(Target),
    /// Hold the queue: window closed or rate budget exhausted.
    Hold { reason: String },
    /// Platform answered 429: back off this many seconds before retrying
    /// the *platform* call. Probe launches are unaffected unless the
    /// window/rate budget says otherwise.
    Backoff { secs: u64 },
}

/// Token bucket for the platform rate limit.
#[derive(Debug)]
struct Bucket {
    capacity: u32,
    tokens: f64,
    refill_per_sec: f64,
    last_refill: u64,
}

impl Bucket {
    fn new(limit_per_min: u32, burst: u32, now: u64) -> Self {
        Bucket {
            capacity: burst.max(1),
            tokens: burst.max(1) as f64,
            refill_per_sec: limit_per_min as f64 / 60.0,
            last_refill: now,
        }
    }

    fn take(&mut self, now: u64) -> bool {
        let elapsed = now.saturating_sub(self.last_refill) as f64;
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity as f64);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Schedules probe launches against a program's rules of engagement.
/// `C` is the clock; tests use `ManualClock`.
pub struct Scheduler<C: Clock> {
    program: Program,
    max_concurrent: u32,
    clock: C,
    in_flight: u32,
    bucket: Bucket,
    backoff_until: u64,
    next_backoff_secs: u64,
}

impl<C: Clock> Scheduler<C> {
    /// `max_concurrent == 0` is refused: zero means "fail closed", never
    /// "unlimited".
    pub fn new(program: Program, max_concurrent: u32, clock: C) -> Result<Self, String> {
        if max_concurrent == 0 {
            return Err("bounty: max_concurrent must be >= 1".to_string());
        }
        let now = clock.now();
        let bucket = Bucket::new(
            program.rate_limit.requests_per_minute,
            program.rate_limit.burst,
            now,
        );
        Ok(Scheduler {
            program,
            max_concurrent,
            clock,
            in_flight: 0,
            bucket,
            backoff_until: 0,
            next_backoff_secs: 1,
        })
    }

    /// Record a platform 429. Backoff is exponential (1,2,4,…) capped
    /// at 300s; a success resets it via `note_platform_success`.
    pub fn note_platform_429(&mut self) {
        let now = self.clock.now();
        self.backoff_until = now + self.next_backoff_secs;
        self.next_backoff_secs = (self.next_backoff_secs * 2).min(300);
    }

    pub fn note_platform_success(&mut self) {
        self.next_backoff_secs = 1;
    }

    pub fn note_run_finished(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
    }

    pub fn in_flight(&self) -> u32 {
        self.in_flight
    }

    /// Decide launches for one tick. `queue` supplies candidates in
    /// order; at most `max_concurrent - in_flight` launches are emitted,
    /// and none when the window is closed or the rate budget is empty.
    /// `drain` pops launched targets from the caller's queue.
    pub fn tick<D>(&mut self, drain: &mut D) -> Vec<SchedAction>
    where
        D: FnMut() -> Option<Target>,
    {
        let now = self.clock.now();
        if now < self.backoff_until {
            return vec![SchedAction::Backoff {
                secs: self.backoff_until - now,
            }];
        }
        if !self.program.window.allows(now) {
            return vec![SchedAction::Hold {
                reason: "testing window closed".to_string(),
            }];
        }
        let mut actions = Vec::new();
        while self.in_flight < self.max_concurrent {
            if !self.bucket.take(now) {
                actions.push(SchedAction::Hold {
                    reason: "rate budget exhausted".to_string(),
                });
                break;
            }
            match drain() {
                Some(t) => {
                    self.in_flight += 1;
                    actions.push(SchedAction::Launch(t));
                }
                None => break,
            }
        }
        actions
    }

    /// Convenience: launch bookkeeping when the driver spawns the real
    /// child itself.
    pub fn launched_run(&self, target: &Target, run_id: &str, approval_nonce: u64) -> Run {
        Run {
            id: run_id.to_string(),
            target_id: target.id.clone(),
            state: RunState::Running,
            approval_nonce,
            cancel_reason: None,
        }
    }
}
