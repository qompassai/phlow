//! Typed errors for the Mojo worker harness.
//!
//! Every variant carries bounded context (ids, counts, limits). No payload
//! bytes, no worker internals.

use std::fmt::{Display, Formatter, Result as FmtResult};

/// Every way a harness operation can be rejected.
///
/// Rejection never mutates harness state: the queue, in-flight map, results,
/// and generation counter are exactly as they were before the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerError {
    /// The harness is not running (stopped or draining).
    NotRunning,
    /// `start` was called while already running.
    AlreadyRunning,
    /// The intake queue is full; the task was not admitted.
    QueueFull {
        /// Queue capacity.
        capacity: usize,
    },
    /// The task payload exceeds the configured byte budget.
    PayloadTooLarge {
        /// Payload bytes supplied.
        bytes: usize,
        /// Configured maximum.
        max: usize,
    },
    /// No queued or in-flight task has this id.
    UnknownTask {
        /// The requested task id.
        id: u64,
    },
    /// A result arrived for a stale generation and was dropped.
    StaleResult {
        /// Task id of the dropped result.
        id: u64,
        /// Generation the result carried.
        generation: u64,
    },
    /// The worker refused to initialize.
    InitFailed {
        /// Bounded reason.
        reason: String,
    },
    /// The worker configuration itself is invalid.
    InvalidConfig {
        /// Bounded reason.
        reason: String,
    },
    /// A result's output exceeds the result byte budget.
    ResultTooLarge {
        /// Output bytes supplied.
        bytes: usize,
        /// Maximum allowed.
        max: usize,
    },
    /// All 2^64 task ids for this harness lifetime are used; no new task
    /// can be admitted without reusing an id.
    TaskIdsExhausted,
}

impl Display for WorkerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::NotRunning => write!(f, "worker harness is not running"),
            Self::AlreadyRunning => write!(f, "worker harness is already running"),
            Self::QueueFull { capacity } => {
                write!(f, "intake queue full (capacity {capacity})")
            }
            Self::PayloadTooLarge { bytes, max } => {
                write!(f, "task payload is {bytes} bytes, max {max}")
            }
            Self::UnknownTask { id } => write!(f, "unknown task id {id}"),
            Self::StaleResult { id, generation } => write!(
                f,
                "stale result for task {id} (generation {generation}) dropped"
            ),
            Self::InitFailed { reason } => write!(f, "worker init failed: {reason}"),
            Self::InvalidConfig { reason } => write!(f, "invalid worker config: {reason}"),
            Self::ResultTooLarge { bytes, max } => {
                write!(f, "worker result is {bytes} bytes, max {max}")
            }
            Self::TaskIdsExhausted => write!(f, "all 2^64 task ids are used"),
        }
    }
}

impl std::error::Error for WorkerError {}
