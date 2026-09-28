//! Typed worker errors. Every lifecycle misuse names the state it found.

use phlow_compute_cuda::CudaError;
use std::fmt;

/// Failures of the worker lifecycle. Backend failures surface unchanged
/// inside [`WorkerError::Backend`] so callers can match on them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerError {
    /// A job is already running; the worker takes one at a time.
    AlreadyRunning {
        /// The job currently holding the worker.
        job_id: u64,
    },
    /// The operation needs a running job, but the worker is idle.
    NotRunning,
    /// The operation needs a live worker, but it was stopped.
    Stopped,
    /// The handle belongs to a generation the worker already cancelled.
    StaleHandle {
        /// The worker's current generation.
        current: u64,
        /// The generation the handle was issued in.
        got: u64,
    },
    /// The handle names a different job than the one running.
    WrongJob {
        /// The running job's id.
        running: u64,
        /// The id the handle named.
        got: u64,
    },
    /// The job-id counter wrapped past `u64::MAX`.
    JobIdExhausted,
    /// The backend rejected the launch; the worker is idle again.
    Backend(CudaError),
}

impl fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning { job_id } => {
                write!(formatter, "worker is busy with job {job_id}")
            }
            Self::NotRunning => write!(formatter, "worker has no running job"),
            Self::Stopped => write!(formatter, "worker is stopped"),
            Self::StaleHandle { current, got } => write!(
                formatter,
                "handle is from generation {got}; worker is at generation {current}"
            ),
            Self::WrongJob { running, got } => write!(
                formatter,
                "handle names job {got}; worker is running job {running}"
            ),
            Self::JobIdExhausted => write!(formatter, "job-id counter exhausted"),
            Self::Backend(error) => write!(formatter, "backend failed: {error}"),
        }
    }
}

impl std::error::Error for WorkerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CudaError> for WorkerError {
    fn from(error: CudaError) -> Self {
        Self::Backend(error)
    }
}
