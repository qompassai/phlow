//! The Rust-side harness around a [`MojoWorker`](crate::MojoWorker).
//!
//! [`WorkerHarness`] owns everything the worker does not: the lifecycle
//! state machine, the bounded intake queue, task ids, generation tokens,
//! the in-flight map, and the completed-result queue. The worker owns only
//! its internal state and is driven by polling.
//!
//! # Lifecycle
//!
//! `Stopped -> start() -> Running -> drain()/stop() -> Stopped`. `drain`
//! first refuses new submissions (`Draining`), pumps the queue and
//! in-flight map until empty, force-cancels leftovers after
//! [`DRAIN_ROUNDS_MAX`] no-progress rounds, then shuts the worker down.
//! `restart` drains, bumps the generation, and starts again.
//!
//! # Cancellation policy
//!
//! `cancel(id)` removes a queued task immediately (publishing a `Cancelled`
//! result) or marks an in-flight task cancelled and notifies the worker.
//! Results are published only when their task id is in flight *and* their
//! generation matches the harness generation; anything else is dropped and
//! counted as stale. Cancellation never resurrects a finished task and
//! never publishes twice for one task.
//!
//! # Full-queue policy
//!
//! The intake queue rejects: `submit` on a full queue returns
//! [`WorkerError::QueueFull`] and admits nothing. The result queue
//! replaces the oldest: a completed result arriving at a full result queue
//! drops the oldest result and increments a visible counter — results are
//! retrievable, so loss is observable, never silent.

use std::collections::{BTreeMap, VecDeque};

use crate::boundary::{MojoWorker, PollOutcome};
use crate::error::WorkerError;
use crate::types::{TaskId, TaskKind, TaskStatus, WorkerConfig, WorkerResult, WorkerTask};

/// Completed-result slots. Bounded so a runaway worker cannot grow memory
/// without bound; overflow drops the oldest and counts it.
pub const RESULTS_MAX: usize = 64;
/// Tasks admitted from the queue per `pump` call.
pub const PUMP_TASKS_MAX: usize = 64;
/// Pump rounds with zero progress before `drain` force-cancels leftovers.
pub const DRAIN_ROUNDS_MAX: u32 = 4096;

/// Lifecycle states of the harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessState {
    /// Not running. `start` is the only valid transition.
    Stopped,
    /// Accepting submissions and pumping tasks.
    Running,
    /// Refusing submissions; draining queue and in-flight tasks.
    Draining,
}

/// What one `pump` call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PumpReport {
    /// Tasks admitted from the queue this pump.
    pub admitted: usize,
    /// Tasks completed this pump.
    pub completed: usize,
    /// Tasks cancelled this pump.
    pub cancelled: usize,
    /// Stale results dropped this pump.
    pub stale_dropped: usize,
}

/// What one `drain` call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrainReport {
    /// Pump rounds executed.
    pub rounds: u32,
    /// Tasks completed during the drain.
    pub completed: usize,
    /// Tasks cancelled during the drain (including force-cancelled).
    pub cancelled: usize,
}

/// An in-flight task: the task plus whether cancellation was requested.
#[derive(Debug, Clone)]
struct Inflight {
    task: WorkerTask,
    cancelled: bool,
}

/// Per-pump outcome counters shared by the repoll and admit phases.
#[derive(Debug, Default)]
struct OutcomeCounts {
    completed: usize,
    cancelled: usize,
    stale: usize,
}

/// The harness around a Mojo worker.
///
/// Owns: the worker, the config, lifecycle state, the generation counter,
/// the task-id counter, the intake queue, the in-flight map, the result
/// queue, and the drop counters. `W` owns only its internal worker state.
pub struct WorkerHarness<W: MojoWorker> {
    worker: W,
    config: WorkerConfig,
    state: HarnessState,
    generation: u64,
    next_task_id: u64,
    queue: VecDeque<WorkerTask>,
    inflight: BTreeMap<TaskId, Inflight>,
    results: VecDeque<WorkerResult>,
    results_dropped: u64,
    stale_results_dropped: u64,
}

impl<W: MojoWorker> std::fmt::Debug for WorkerHarness<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The worker itself is omitted: implementors are not required to
        // be Debug, and worker internals are not harness state.
        f.debug_struct("WorkerHarness")
            .field("state", &self.state)
            .field("generation", &self.generation)
            .field("queue_len", &self.queue.len())
            .field("inflight_len", &self.inflight.len())
            .field("results_len", &self.results.len())
            .field("results_dropped", &self.results_dropped)
            .field("stale_results_dropped", &self.stale_results_dropped)
            .finish_non_exhaustive()
    }
}

impl<W: MojoWorker> WorkerHarness<W> {
    /// Build a stopped harness around `worker`.
    #[must_use]
    pub fn new(worker: W, config: WorkerConfig) -> Self {
        Self {
            worker,
            config,
            state: HarnessState::Stopped,
            generation: 0,
            next_task_id: 0,
            queue: VecDeque::new(),
            inflight: BTreeMap::new(),
            results: VecDeque::new(),
            results_dropped: 0,
            stale_results_dropped: 0,
        }
    }

    /// Read-only access to the wrapped worker (e.g. for worker-side stats).
    #[must_use]
    pub fn worker(&self) -> &W {
        &self.worker
    }

    /// Current lifecycle state.
    #[must_use]
    pub fn state(&self) -> HarnessState {
        self.state
    }

    /// Current generation (bumped by `restart`).
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Queued (not yet admitted) task count.
    #[must_use]
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// In-flight task count.
    #[must_use]
    pub fn inflight_len(&self) -> usize {
        self.inflight.len()
    }

    /// Completed results waiting for `take_result`.
    #[must_use]
    pub fn results_len(&self) -> usize {
        self.results.len()
    }

    /// Results dropped by result-queue overflow.
    #[must_use]
    pub fn results_dropped(&self) -> u64 {
        self.results_dropped
    }

    /// Results dropped as stale (wrong id or generation).
    #[must_use]
    pub fn stale_results_dropped(&self) -> u64 {
        self.stale_results_dropped
    }

    /// Start the harness: initialize the worker and enter `Running`.
    ///
    /// # Errors
    ///
    /// [`WorkerError::AlreadyRunning`] when not stopped; the worker's own
    /// error when `init` fails (the harness stays stopped).
    pub fn start(&mut self) -> Result<(), WorkerError> {
        if self.state != HarnessState::Stopped {
            return Err(WorkerError::AlreadyRunning);
        }
        self.worker.init(&self.config)?;
        self.state = HarnessState::Running;
        Ok(())
    }

    /// Admit a task: validate, stamp id + generation, enqueue.
    ///
    /// # Errors
    ///
    /// [`WorkerError::NotRunning`] when not running;
    /// [`WorkerError::PayloadTooLarge`] before any queue admission;
    /// [`WorkerError::QueueFull`] when the queue is at capacity. Rejection
    /// changes nothing.
    pub fn submit(&mut self, kind: TaskKind, payload: Vec<u8>) -> Result<TaskId, WorkerError> {
        if self.state != HarnessState::Running {
            return Err(WorkerError::NotRunning);
        }
        if payload.len() > self.config.payload_bytes_max() {
            return Err(WorkerError::PayloadTooLarge {
                bytes: payload.len(),
                max: self.config.payload_bytes_max(),
            });
        }
        if self.queue.len() >= self.config.queue_capacity() {
            return Err(WorkerError::QueueFull {
                capacity: self.config.queue_capacity(),
            });
        }
        let id = TaskId::next(self.next_task_id);
        // 2^64 task ids per harness lifetime is the bound; saturation keeps
        // ids monotonic rather than wrapping into collision.
        self.next_task_id = self.next_task_id.saturating_add(1);
        self.queue
            .push_back(WorkerTask::new(id, self.generation, kind, payload));
        Ok(id)
    }

    /// Cancel a queued or in-flight task.
    ///
    /// Queued tasks are removed and a `Cancelled` result is published
    /// immediately. In-flight tasks are marked cancelled and the worker is
    /// notified; the `Cancelled` result is published on the next pump.
    ///
    /// # Errors
    ///
    /// [`WorkerError::NotRunning`] when stopped;
    /// [`WorkerError::UnknownTask`] when no queued or in-flight task has
    /// the id (including already-completed tasks: cancellation cannot
    /// resurrect them).
    pub fn cancel(&mut self, id: TaskId) -> Result<(), WorkerError> {
        if self.state == HarnessState::Stopped {
            return Err(WorkerError::NotRunning);
        }
        if let Some(position) = self.queue.iter().position(|task| task.id() == id) {
            if let Some(task) = self.queue.remove(position) {
                self.publish_cancelled(task.id(), task.generation());
            }
            return Ok(());
        }
        if let Some(entry) = self.inflight.get_mut(&id) {
            entry.cancelled = true;
            self.worker.cancel(id, self.generation);
            return Ok(());
        }
        Err(WorkerError::UnknownTask { id: id.get() })
    }

    /// Drive progress: repoll in-flight tasks, then admit queued tasks.
    ///
    /// Bounded: at most [`PUMP_TASKS_MAX`] admissions per call, and the
    /// repoll phase visits each in-flight task once.
    ///
    /// # Errors
    ///
    /// [`WorkerError::NotRunning`] when not running; the worker's error
    /// when a poll fails (the affected task is kept, nothing is lost).
    pub fn pump(&mut self) -> Result<PumpReport, WorkerError> {
        if self.state != HarnessState::Running {
            return Err(WorkerError::NotRunning);
        }
        self.pump_once()
    }

    /// Take the oldest completed result, if any.
    #[must_use]
    pub fn take_result(&mut self) -> Option<WorkerResult> {
        self.results.pop_front()
    }

    /// Drain the harness: refuse new submissions, pump until the queue and
    /// in-flight map are empty, force-cancel leftovers after
    /// [`DRAIN_ROUNDS_MAX`] no-progress rounds, shut the worker down.
    ///
    /// Always shuts the worker down and returns to `Stopped`, even when a
    /// poll fails mid-drain (the error is still reported to the caller).
    pub fn drain(&mut self) -> Result<DrainReport, WorkerError> {
        if self.state == HarnessState::Stopped {
            return Ok(DrainReport::default());
        }
        self.state = HarnessState::Draining;
        let outcome = self.drain_loop();
        self.worker.shutdown();
        self.state = HarnessState::Stopped;
        outcome
    }

    /// Drain, bump the generation, and start again.
    ///
    /// Results from the previous generation can never publish afterwards:
    /// the drain empties the in-flight map, and the publish path checks the
    /// generation anyway.
    ///
    /// # Errors
    ///
    /// The drain's error, or the worker's error when re-`init` fails.
    pub fn restart(&mut self) -> Result<(), WorkerError> {
        self.drain()?;
        // Saturates rather than wraps: stale-generation checks use exact
        // equality, so saturation cannot resurrect an old result.
        self.generation = self.generation.saturating_add(1);
        self.start()
    }

    /// One pump round without the running-state check (used by `drain`).
    fn pump_once(&mut self) -> Result<PumpReport, WorkerError> {
        let mut counts = OutcomeCounts::default();
        self.repoll_inflight(&mut counts)?;
        let admitted = self.admit_queued(PUMP_TASKS_MAX, &mut counts)?;
        Ok(PumpReport {
            admitted,
            completed: counts.completed,
            cancelled: counts.cancelled,
            stale_dropped: counts.stale,
        })
    }

    /// Poll every in-flight task once.
    fn repoll_inflight(&mut self, counts: &mut OutcomeCounts) -> Result<(), WorkerError> {
        // Snapshot the ids: the map is mutated as outcomes settle.
        let ids: Vec<TaskId> = self.inflight.keys().copied().collect();
        for id in ids {
            let Some(entry) = self.inflight.remove(&id) else {
                continue;
            };
            match self.worker.poll(&entry.task) {
                Err(error) => {
                    // Keep the task: the failure is reported, nothing lost.
                    self.inflight.insert(id, entry);
                    return Err(error);
                }
                Ok(outcome) => self.settle(&entry.task, entry.cancelled, outcome, counts),
            }
        }
        Ok(())
    }

    /// Admit up to `budget` queued tasks, polling each immediately.
    fn admit_queued(
        &mut self,
        budget: usize,
        counts: &mut OutcomeCounts,
    ) -> Result<usize, WorkerError> {
        let mut admitted = 0;
        while admitted < budget {
            let Some(task) = self.queue.pop_front() else {
                break;
            };
            match self.worker.poll(&task) {
                Err(error) => {
                    // Return the task to the queue head: admission failed,
                    // the task is not lost.
                    self.queue.push_front(task);
                    return Err(error);
                }
                Ok(outcome) => self.settle(&task, false, outcome, counts),
            }
            admitted += 1;
        }
        Ok(admitted)
    }

    /// Settle one poll outcome: requeue pending, publish or drop the rest.
    fn settle(
        &mut self,
        task: &WorkerTask,
        was_cancelled: bool,
        outcome: PollOutcome,
        counts: &mut OutcomeCounts,
    ) {
        let id = task.id();
        match outcome {
            PollOutcome::Pending => {
                self.inflight.insert(
                    id,
                    Inflight {
                        task: task.clone(),
                        cancelled: was_cancelled,
                    },
                );
            }
            PollOutcome::Ready(result) => {
                // A cancelled task's Ready is dropped: cancellation wins.
                // A result for the wrong id or generation is stale: dropped
                // and counted, never published.
                if was_cancelled {
                    self.publish_cancelled(id, task.generation());
                    counts.cancelled += 1;
                } else if result.task_id() == id && result.generation() == self.generation {
                    self.publish_result(result);
                    counts.completed += 1;
                } else {
                    self.stale_results_dropped += 1;
                    counts.stale += 1;
                }
            }
            PollOutcome::Cancelled(_) => {
                self.publish_cancelled(id, task.generation());
                counts.cancelled += 1;
            }
        }
    }

    /// Publish a result, dropping the oldest on overflow (counted).
    fn publish_result(&mut self, result: WorkerResult) {
        if self.results.len() >= RESULTS_MAX {
            self.results.pop_front();
            self.results_dropped += 1;
        }
        self.results.push_back(result);
    }

    /// Publish a `Cancelled` result for a task.
    fn publish_cancelled(&mut self, id: TaskId, generation: u64) {
        // Empty output cannot exceed the budget; the error arm is
        // unreachable, so fall back to dropping the result instead of
        // panicking.
        if let Ok(result) = WorkerResult::new(id, generation, TaskStatus::Cancelled, Vec::new()) {
            self.publish_result(result);
        }
    }

    /// Pump until empty or the no-progress round cap, then force-cancel
    /// leftovers.
    fn drain_loop(&mut self) -> Result<DrainReport, WorkerError> {
        let mut report = DrainReport::default();
        while !self.queue.is_empty() || !self.inflight.is_empty() {
            if report.rounds >= DRAIN_ROUNDS_MAX {
                break;
            }
            let pump = self.pump_once()?;
            report.rounds += 1;
            report.completed += pump.completed;
            report.cancelled += pump.cancelled;
            let progressed = pump.admitted + pump.completed + pump.cancelled + pump.stale_dropped;
            if progressed == 0 {
                break;
            }
        }
        report.cancelled += self.force_cancel_leftovers();
        Ok(report)
    }

    /// Cancel everything still queued or in flight, publishing `Cancelled`
    /// results. Returns the number of tasks force-cancelled.
    fn force_cancel_leftovers(&mut self) -> usize {
        let mut cancelled = 0;
        while let Some(task) = self.queue.pop_front() {
            self.publish_cancelled(task.id(), task.generation());
            cancelled += 1;
        }
        let ids: Vec<TaskId> = self.inflight.keys().copied().collect();
        for id in ids {
            if let Some(entry) = self.inflight.remove(&id) {
                self.worker.cancel(id, self.generation);
                self.publish_cancelled(id, entry.task.generation());
                cancelled += 1;
            }
        }
        cancelled
    }
}
