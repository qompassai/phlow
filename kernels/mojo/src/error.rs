//! Typed errors for Mojo kernel contracts.
//!
//! Every variant carries bounded context (numbers, names, limits) so a caller
//! can decide what to do. Raw payloads and device internals never appear in
//! messages.

use std::fmt::{Display, Formatter, Result as FmtResult};

/// Axis of a [`crate::Dim3`] that failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimAxis {
    /// The x component.
    X,
    /// The y component.
    Y,
    /// The z component.
    Z,
}

/// Every way a kernel descriptor, launch config, budget, or plan can be
/// rejected. Rejection never mutates published state (registry, planner
/// counters); the caller observes the error and retries with fixed input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelError {
    /// Kernel name is empty.
    NameEmpty,
    /// Kernel name exceeds the character budget.
    NameTooLong {
        /// Characters supplied.
        chars: usize,
        /// Maximum allowed.
        max: usize,
    },
    /// Kernel name contains a character outside `[A-Za-z0-9_]`.
    NameInvalidChar {
        /// The offending character.
        ch: char,
    },
    /// A kernel with this name is already registered; the original is kept.
    DuplicateKernel {
        /// The colliding name.
        name: String,
    },
    /// No kernel with this name is registered.
    UnknownKernel {
        /// The requested name.
        name: String,
    },
    /// The registry already holds [`crate::KERNELS_MAX`] kernels.
    RegistryFull {
        /// The registry capacity.
        max: usize,
    },
    /// A grid or block dimension is zero on the given axis.
    ZeroDim {
        /// Which axis is zero.
        axis: DimAxis,
    },
    /// A grid or block dimension exceeds the per-axis device limit.
    DimTooLarge {
        /// Which axis is too large.
        axis: DimAxis,
        /// The supplied value.
        value: u32,
        /// The device limit.
        max: u32,
    },
    /// The block's thread product exceeds the device limit.
    ThreadsPerBlockExceeded {
        /// Requested threads (x*y*z).
        requested: u64,
        /// Device limit.
        limit: u32,
    },
    /// Requested shared memory exceeds the per-block device limit.
    SharedMemoryExceeded {
        /// Requested bytes.
        requested: u32,
        /// Device limit in bytes.
        limit: u32,
    },
    /// The launch asks for less shared memory than the kernel declares.
    SharedMemoryInsufficient {
        /// Bytes the kernel declares it needs.
        required: u32,
        /// Bytes the launch provides.
        provided: u32,
    },
    /// grid-blocks * block-threads exceeds the per-launch thread budget.
    TotalThreadsExceeded {
        /// Requested total threads.
        requested: u64,
        /// Launch budget.
        limit: u64,
    },
    /// A dimension product overflowed u64 before any limit could apply.
    ArithmeticOverflow {
        /// What was being computed, e.g. "block thread product".
        what: &'static str,
    },
    /// The launch passes more arguments than the kernel declares.
    ArgCountExceeded {
        /// Arguments supplied.
        requested: u8,
        /// Kernel's declared maximum.
        max: u8,
    },
    /// The launch payload exceeds the kernel's (or global) byte budget.
    PayloadBytesExceeded {
        /// Payload bytes supplied.
        requested: u32,
        /// Applicable limit in bytes.
        limit: u32,
    },
    /// The planner's launch budget is exhausted.
    LaunchesExceeded {
        /// Launches already committed.
        used: u32,
        /// Budget maximum.
        max: u32,
    },
    /// The planner's payload byte budget would be exceeded.
    BudgetBytesExceeded {
        /// Bytes already committed plus requested.
        requested: u64,
        /// Budget maximum in bytes.
        max: u64,
    },
    /// The executor refused a well-formed plan (device-side failure).
    ExecutorRejected {
        /// Bounded reason (truncated to [`crate::REASON_CHARS_MAX`]).
        reason: String,
    },
}

impl Display for KernelError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::NameEmpty => write!(f, "kernel name is empty"),
            Self::NameTooLong { chars, max } => {
                write!(f, "kernel name has {chars} chars, max {max}")
            }
            Self::NameInvalidChar { ch } => {
                write!(f, "kernel name contains invalid char {ch:?}")
            }
            Self::DuplicateKernel { name } => {
                write!(f, "kernel {name:?} is already registered")
            }
            Self::UnknownKernel { name } => {
                write!(f, "unknown kernel {name:?}")
            }
            Self::RegistryFull { max } => {
                write!(f, "kernel registry full ({max} kernels)")
            }
            Self::ZeroDim { axis } => {
                write!(f, "dimension is zero on axis {axis:?}")
            }
            Self::DimTooLarge { axis, value, max } => {
                write!(f, "dimension {value} on axis {axis:?} exceeds max {max}")
            }
            Self::ThreadsPerBlockExceeded { requested, limit } => {
                write!(f, "block requests {requested} threads, device max {limit}")
            }
            Self::SharedMemoryExceeded { requested, limit } => write!(
                f,
                "launch requests {requested} shared bytes, device max {limit}"
            ),
            Self::SharedMemoryInsufficient { required, provided } => write!(
                f,
                "kernel needs {required} shared bytes, launch provides {provided}"
            ),
            Self::TotalThreadsExceeded { requested, limit } => write!(
                f,
                "launch requests {requested} total threads, budget max {limit}"
            ),
            Self::ArithmeticOverflow { what } => {
                write!(f, "overflow computing {what}")
            }
            Self::ArgCountExceeded { requested, max } => {
                write!(f, "launch passes {requested} args, kernel max {max}")
            }
            Self::PayloadBytesExceeded { requested, limit } => {
                write!(f, "payload is {requested} bytes, limit {limit}")
            }
            Self::LaunchesExceeded { used, max } => {
                write!(f, "launch budget exhausted ({used}/{max} used)")
            }
            Self::BudgetBytesExceeded { requested, max } => {
                write!(f, "payload budget would reach {requested} bytes, max {max}")
            }
            Self::ExecutorRejected { reason } => {
                write!(f, "executor rejected launch: {reason}")
            }
        }
    }
}

impl std::error::Error for KernelError {}
