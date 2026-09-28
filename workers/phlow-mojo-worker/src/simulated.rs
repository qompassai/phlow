//! Simulated Mojo worker (`simulated` feature).
//!
//! [`SimulatedMojoWorker`] implements [`MojoWorker`](crate::MojoWorker) in
//! ordinary Rust so the harness contract is exercisable in tests without
//! Mojo. It honors every boundary obligation faithfully:
//!
//! - bounded work per poll (one task step per call),
//! - prompt cancellation (a cancelled task's next poll yields `Cancelled`),
//! - configurable pending polls before completion (models async work),
//! - deterministic output (byte-wise wrapping add of 1 over the payload),
//! - outputs within [`crate::RESULT_BYTES_MAX`] by construction (output
//!   length equals input length, and inputs are payload-budgeted),
//! - an optional one-shot init failure and an optional one-shot poll
//!   failure, for failure-path tests.
//!
//! It is a test double, not a Mojo worker: no Mojo is compiled, no model
//! runs, no device is touched.

use std::collections::{HashMap, HashSet};

use crate::boundary::{MojoWorker, PollOutcome};
use crate::error::WorkerError;
use crate::types::{TaskId, TaskStatus, WorkerConfig, WorkerResult, WorkerTask};

/// How many `Pending` polls a task sees before it completes.
pub const PENDING_POLLS_MAX: u32 = 1024;

/// In-process test double for the Mojo worker boundary.
#[derive(Debug, Default)]
pub struct SimulatedMojoWorker {
    initialized: bool,
    shutdown_called: bool,
    polls_before_ready: u32,
    pending: HashMap<(TaskId, u64), u32>,
    cancelled: HashSet<(TaskId, u64)>,
    fail_init: bool,
    fail_poll: bool,
    poll_count: u64,
}

impl SimulatedMojoWorker {
    /// A worker that completes tasks on the first poll.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A worker that returns `Pending` `polls` times per task before
    /// completing it (models asynchronous Mojo work).
    ///
    /// `polls` is capped at [`PENDING_POLLS_MAX`].
    #[must_use]
    pub fn with_pending_polls(polls: u32) -> Self {
        Self {
            polls_before_ready: polls.min(PENDING_POLLS_MAX),
            ..Self::default()
        }
    }

    /// Make the next `init` fail. One-shot: consumed by the failing call.
    pub fn fail_next_init(&mut self) {
        self.fail_init = true;
    }

    /// Make the next `poll` fail with a worker error. One-shot.
    pub fn fail_next_poll(&mut self) {
        self.fail_poll = true;
    }

    /// Whether `shutdown` was called.
    #[must_use]
    pub fn shutdown_called(&self) -> bool {
        self.shutdown_called
    }

    /// Total `poll` calls served.
    #[must_use]
    pub fn poll_count(&self) -> u64 {
        self.poll_count
    }

    /// Deterministic transform: each payload byte wrapping-adds 1.
    /// Bounded: output length equals input length.
    fn transform(payload: &[u8]) -> Vec<u8> {
        payload.iter().map(|byte| byte.wrapping_add(1)).collect()
    }
}

impl MojoWorker for SimulatedMojoWorker {
    fn init(&mut self, _config: &WorkerConfig) -> Result<(), WorkerError> {
        if self.fail_init {
            self.fail_init = false;
            return Err(WorkerError::InitFailed {
                reason: "simulated init failure".to_owned(),
            });
        }
        self.initialized = true;
        Ok(())
    }

    fn poll(&mut self, task: &WorkerTask) -> Result<PollOutcome, WorkerError> {
        self.poll_count += 1;
        if self.fail_poll {
            self.fail_poll = false;
            return Err(WorkerError::InitFailed {
                reason: "simulated poll failure".to_owned(),
            });
        }
        let key = (task.id(), task.generation());
        if self.cancelled.contains(&key) {
            return Ok(PollOutcome::Cancelled(task.id()));
        }
        let seen = self.pending.entry(key).or_insert(0);
        if *seen < self.polls_before_ready {
            *seen += 1;
            return Ok(PollOutcome::Pending);
        }
        self.pending.remove(&key);
        // Output length equals input length, and the harness budgets inputs
        // below RESULT_BYTES_MAX, so this cannot exceed the budget.
        let output = Self::transform(task.payload());
        let result =
            WorkerResult::new(task.id(), task.generation(), TaskStatus::Completed, output)?;
        Ok(PollOutcome::Ready(result))
    }

    fn cancel(&mut self, task_id: TaskId, generation: u64) {
        self.cancelled.insert((task_id, generation));
    }

    fn shutdown(&mut self) {
        self.shutdown_called = true;
        self.initialized = false;
    }
}
