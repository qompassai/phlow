//! The Mojo worker interface boundary.
//!
//! [`MojoWorker`] is the contract a Mojo-implemented worker must satisfy to
//! plug into phlow's agent harness. It is a trait, not FFI: a real Mojo
//! implementation would expose these entry points through Mojo's declared
//! bindings (Mojo compiles to native code and can be called from the host
//! through explicitly declared interfaces), and a thin Rust adapter would
//! implement this trait over them. No Mojo compiles in this workspace; the
//! trait *is* the boundary, documented precisely so the Mojo side can be
//! written against it later.
//!
//! # Mojo-side obligations
//!
//! A conforming Mojo worker must:
//!
//! - implement `init` (allocate worker state; validate the config; return a
//!   bounded error string on failure — never trap),
//! - implement `poll` as a bounded unit of work: inspect the task, do at
//!   most one scheduling quantum, and return `Pending`, `Ready`, or
//!   `Cancelled`. It must not block indefinitely; the harness drives
//!   progress by polling.
//! - honor `cancel` promptly: the next `poll` for the named task returns
//!   `Cancelled`, and no `Ready` is produced for it afterwards,
//! - produce outputs within [`crate::RESULT_BYTES_MAX`] and never retain
//!   the task payload beyond the poll that consumes it (the harness owns
//!   the payload bytes),
//! - implement `shutdown` as idempotent cleanup that releases worker state.
//!
//! The harness never calls `poll` concurrently for the same worker: the
//! Mojo side needs no internal locking for the poll path.

use crate::error::WorkerError;
use crate::types::{TaskId, WorkerConfig, WorkerResult, WorkerTask};

/// One poll step's outcome for a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    /// The task needs more polling; no result yet.
    Pending,
    /// The task finished; the result carries the task's id and generation.
    Ready(WorkerResult),
    /// The task was cancelled; no result will follow.
    Cancelled(TaskId),
}

/// The interface a Mojo-implemented worker exposes to the harness.
///
/// The implementor owns all worker-side state (model weights, thread pools,
/// device handles). The harness owns task payloads, ids, generations, and
/// the result queue. Results with a generation or id the harness did not
/// issue are dropped as stale — the worker cannot publish on its own
/// authority.
pub trait MojoWorker {
    /// Initialize worker state from the operator config.
    ///
    /// Called once per harness generation, before any `poll`. On `Err` the
    /// harness stays stopped and reports [`WorkerError::InitFailed`]-style
    /// context to the operator.
    fn init(&mut self, config: &WorkerConfig) -> Result<(), WorkerError>;

    /// Perform a bounded unit of work on `task`.
    ///
    /// The task is borrowed: the worker must not retain it. Returning
    /// `Ready` transfers the result to the harness; returning `Pending`
    /// keeps the task in flight for a later poll.
    fn poll(&mut self, task: &WorkerTask) -> Result<PollOutcome, WorkerError>;

    /// Request cancellation of an in-flight task.
    ///
    /// Best-effort and idempotent: unknown or already-finished ids are
    /// ignored. The next `poll` for the task must yield
    /// [`PollOutcome::Cancelled`].
    fn cancel(&mut self, task_id: TaskId, generation: u64);

    /// Release worker state. Idempotent; safe to call on a stopped worker.
    fn shutdown(&mut self);
}
