//! Task, result, and configuration types for the Mojo worker harness.
//!
//! All types crossing the harness/worker boundary are validated at
//! construction: payloads and outputs are byte-bounded, configs reject
//! degenerate values, and task ids are assigned by the harness (never by the
//! caller) so they cannot collide.

use std::time::Duration;

use crate::error::WorkerError;

/// Maximum task payload bytes (1 MiB). The operator may configure less.
pub const PAYLOAD_BYTES_MAX: usize = 1024 * 1024;
/// Maximum worker result output bytes (4 MiB).
pub const RESULT_BYTES_MAX: usize = 4 * 1024 * 1024;
/// Maximum intake queue capacity an operator may configure.
pub const QUEUE_TASKS_MAX: usize = 1024;
/// Maximum characters in a failure reason string.
pub const REASON_CHARS_MAX: usize = 256;
/// Upper bound on a configured task deadline (24 h).
pub const TASK_DEADLINE_MAX: Duration = Duration::from_secs(24 * 3600);

/// Stable identifier for a task, assigned by the harness.
///
/// Wraps a `u64` so task ids cannot be confused with other integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(u64);

impl TaskId {
    /// Build an id from a raw counter. Crate-internal: ids are assigned by
    /// the harness so callers cannot collide with them.
    pub(crate) fn next(raw: u64) -> Self {
        Self(raw)
    }

    /// The numeric id. Public for logging and correlation only; ids are
    /// created by the harness.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "task-{}", self.0)
    }
}

/// The shape of work a Mojo worker can be asked to do.
///
/// These are the offload shapes that motivate a Mojo worker in an agent
/// harness: small, vectorizable, data-parallel transforms over bounded
/// payloads — the same class of work Mojo's SIMD types accelerate on CPU.
/// The enum is deliberately closed: adding a kind is a contract change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
    /// Vectorize a batch of texts into embeddings (payload: length-prefixed
    /// UTF-8 texts; output: length-prefixed f32 vectors).
    Embed,
    /// Score candidate outputs against a reference (payload: reference +
    /// candidates; output: f32 scores).
    Score,
    /// Byte-level deterministic transform (payload in, payload out).
    Transform,
}

/// A unit of work admitted by the harness.
///
/// Constructed only by [`crate::WorkerHarness::submit`]: the id and the
/// harness generation are stamped at admission, so a task always carries the
/// generation it was accepted under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerTask {
    id: TaskId,
    generation: u64,
    kind: TaskKind,
    payload: Vec<u8>,
}

impl WorkerTask {
    /// Build a task. Only the harness calls this; the payload must already
    /// be within the configured budget (checked by the caller).
    pub(crate) fn new(id: TaskId, generation: u64, kind: TaskKind, payload: Vec<u8>) -> Self {
        Self {
            id,
            generation,
            kind,
            payload,
        }
    }

    /// Task id assigned at admission.
    #[must_use]
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Harness generation the task was admitted under.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The kind of work requested.
    #[must_use]
    pub fn kind(&self) -> TaskKind {
        self.kind
    }

    /// The task payload bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// Terminal status of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    /// The worker completed the task.
    Completed,
    /// The worker reported failure (bounded reason).
    Failed {
        /// Bounded failure reason.
        reason: String,
    },
    /// The task was cancelled before completion.
    Cancelled,
}

impl TaskStatus {
    /// Build a `Failed` status with the reason truncated to
    /// [`REASON_CHARS_MAX`] characters.
    #[must_use]
    pub fn failed(reason: &str) -> Self {
        Self::Failed {
            reason: reason.chars().take(REASON_CHARS_MAX).collect(),
        }
    }
}

/// The typed outcome of a task.
///
/// The constructor enforces the output byte budget; results that exceed it
/// are a [`WorkerError::ResultTooLarge`], never silently truncated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerResult {
    task_id: TaskId,
    generation: u64,
    status: TaskStatus,
    output: Vec<u8>,
}

impl WorkerResult {
    /// Build a result, enforcing the output byte budget.
    ///
    /// # Errors
    ///
    /// [`WorkerError::ResultTooLarge`] if `output` exceeds
    /// [`RESULT_BYTES_MAX`].
    pub fn new(
        task_id: TaskId,
        generation: u64,
        status: TaskStatus,
        output: Vec<u8>,
    ) -> Result<Self, WorkerError> {
        if output.len() > RESULT_BYTES_MAX {
            return Err(WorkerError::ResultTooLarge {
                bytes: output.len(),
                max: RESULT_BYTES_MAX,
            });
        }
        Ok(Self {
            task_id,
            generation,
            status,
            output,
        })
    }

    /// The task this result answers.
    #[must_use]
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    /// The generation this result was produced under.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Terminal status.
    #[must_use]
    pub fn status(&self) -> &TaskStatus {
        &self.status
    }

    /// Result output bytes.
    #[must_use]
    pub fn output(&self) -> &[u8] {
        &self.output
    }
}

/// Operator configuration for the harness and its worker.
///
/// Validated at construction: the queue holds at least one task, the payload
/// budget is within the global cap, and the deadline is sane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerConfig {
    queue_capacity: usize,
    payload_bytes_max: usize,
    task_deadline: Duration,
}

impl WorkerConfig {
    /// Build a validated config.
    ///
    /// # Errors
    ///
    /// [`WorkerError::InvalidConfig`] when `queue_capacity` is 0 or above
    /// [`QUEUE_TASKS_MAX`], when `payload_bytes_max` is 0 or above
    /// [`PAYLOAD_BYTES_MAX`], or when `task_deadline` is zero or above
    /// [`TASK_DEADLINE_MAX`].
    pub fn new(
        queue_capacity: usize,
        payload_bytes_max: usize,
        task_deadline: Duration,
    ) -> Result<Self, WorkerError> {
        if queue_capacity == 0 || queue_capacity > QUEUE_TASKS_MAX {
            return Err(WorkerError::InvalidConfig {
                reason: format!("queue_capacity {queue_capacity} not in 1..={QUEUE_TASKS_MAX}"),
            });
        }
        if payload_bytes_max == 0 || payload_bytes_max > PAYLOAD_BYTES_MAX {
            return Err(WorkerError::InvalidConfig {
                reason: format!(
                    "payload_bytes_max {payload_bytes_max} not in 1..={PAYLOAD_BYTES_MAX}"
                ),
            });
        }
        if task_deadline.is_zero() || task_deadline > TASK_DEADLINE_MAX {
            return Err(WorkerError::InvalidConfig {
                reason: format!(
                    "task_deadline {}s not in 1s..={}s",
                    task_deadline.as_secs(),
                    TASK_DEADLINE_MAX.as_secs()
                ),
            });
        }
        Ok(Self {
            queue_capacity,
            payload_bytes_max,
            task_deadline,
        })
    }

    /// Intake queue capacity in tasks.
    #[must_use]
    pub fn queue_capacity(&self) -> usize {
        self.queue_capacity
    }

    /// Per-task payload byte budget.
    #[must_use]
    pub fn payload_bytes_max(&self) -> usize {
        self.payload_bytes_max
    }

    /// Per-task deadline (advisory: the harness stamps it on tasks; a real
    /// Mojo worker enforces it).
    #[must_use]
    pub fn task_deadline(&self) -> Duration {
        self.task_deadline
    }
}
