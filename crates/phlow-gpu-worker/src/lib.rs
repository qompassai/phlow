#![forbid(unsafe_code)]

//! A small GPU work dispatcher with a bounded lifecycle.
//!
//! The idea, adapted from cuda-oxide's host runtimes (`cuda-core` /
//! `cuda-async`): kernel launches go through a worker that owns submission,
//! completion, and cancellation. This crate is CPU-only: the worker drives
//! any [`CudaBackend`][phlow_compute_cuda::CudaBackend], and tests use the
//! deterministic [`SimulatedBackend`][phlow_compute_cuda::SimulatedBackend].
//!
//! Cancellation uses generations: every `cancel` or `stop` bumps a counter,
//! and any [`JobHandle`] from an older generation is rejected as stale.
//! There is no hidden shared state and no threads — the state machine is
//! explicit and every transition is typed.

mod error;
mod job;
mod worker;

pub use error::WorkerError;
pub use job::{Job, JobHandle};
pub use worker::{Worker, WorkerState};
