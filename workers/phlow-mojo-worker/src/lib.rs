//! Experimental Mojo worker for phlow's agent harness.
//!
//! This crate models how a Mojo-implemented worker would plug into phlow:
//! the Rust side owns the harness contract (worker lifecycle, bounded task
//! intake, typed results, cancellation policy) and the Mojo side implements
//! the [`MojoWorker`] interface boundary. Mojo code cannot compile in this
//! Rust workspace, so the boundary is a trait plus a precise obligations
//! list (see [`boundary`]); the `simulated` feature provides
//! [`SimulatedMojoWorker`], an in-process test double that honors the
//! boundary faithfully.
//!
//! # What is real vs. interface
//!
//! - **Real:** [`WorkerHarness`] (lifecycle state machine, bounded intake
//!   queue with reject-on-full policy, task ids, generation tokens,
//!   in-flight tracking, bounded result queue, cancellation policy),
//!   [`WorkerConfig`] validation, [`WorkerTask`] / [`WorkerResult`] /
//!   [`TaskStatus`] typed results, [`WorkerError`].
//! - **Interface:** [`MojoWorker`] is what the Mojo side must implement
//!   (`init`, `poll`, `cancel`, `shutdown`). A real Mojo implementation
//!   would expose these through Mojo's declared bindings; a thin Rust
//!   adapter would implement the trait over them.
//! - **Test double:** [`SimulatedMojoWorker`] (`simulated` feature, on by
//!   default) runs the boundary in ordinary Rust: bounded polls, prompt
//!   cancellation, deterministic output. For tests and interface
//!   experiments only — never a substitute for a Mojo worker.
//!
//! # Adapted Mojo concepts
//!
//! - *Ownership discipline.* Mojo's value-ownership model (inspired by
//!   Rust) maps onto this crate directly: the harness owns task payloads,
//!   ids, and results; the worker owns only its internal state; `poll`
//!   borrows the task and must not retain it.
//! - *Bounded, explicit work.* Mojo kernels declare their launch geometry
//!   up front; here every queue, payload, result, and poll budget is a
//!   named constant, and `poll` performs one bounded step per call.
//! - *Async enqueue + synchronize.* Like Mojo's `enqueue_function` /
//!   `synchronize` split, submission is decoupled from completion: the
//!   harness admits tasks and drives progress with `pump`, collecting
//!   results as they arrive.
//!
//! # Limits
//!
//! Every bound is a named constant next to the code it guards:
//! [`PAYLOAD_BYTES_MAX`], [`RESULT_BYTES_MAX`], [`QUEUE_TASKS_MAX`],
//! [`REASON_CHARS_MAX`], [`TASK_DEADLINE_MAX`], [`RESULTS_MAX`],
//! [`PUMP_TASKS_MAX`], [`DRAIN_ROUNDS_MAX`], [`PENDING_POLLS_MAX`].
//!
//! # Layout
//!
//! [`error`] owns the error type; [`types`] owns tasks, results, and
//! config; [`boundary`] owns the `MojoWorker` trait and its obligations;
//! [`harness`] owns the lifecycle, queues, and cancellation policy;
//! [`simulated`] owns the test double.

#![forbid(unsafe_code)]

pub mod boundary;
pub mod error;
pub mod harness;
#[cfg(feature = "simulated")]
pub mod simulated;
pub mod types;

pub use boundary::{MojoWorker, PollOutcome};
pub use error::WorkerError;
pub use harness::{
    DRAIN_ROUNDS_MAX, DrainReport, HarnessState, PUMP_TASKS_MAX, PumpReport, RESULTS_MAX,
    WorkerHarness,
};
#[cfg(feature = "simulated")]
pub use simulated::{PENDING_POLLS_MAX, SimulatedMojoWorker};
pub use types::{
    PAYLOAD_BYTES_MAX, QUEUE_TASKS_MAX, REASON_CHARS_MAX, RESULT_BYTES_MAX, TASK_DEADLINE_MAX,
    TaskId, TaskKind, TaskStatus, WorkerConfig, WorkerResult, WorkerTask,
};
