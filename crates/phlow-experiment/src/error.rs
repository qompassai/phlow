//! Typed experiment errors. Every misuse names what it found.
//!
//! The style mirrors `phlow-council`: a plain enum with a manual
//! [`std::fmt::Display`] implementation, no external error crate, so every
//! failure is explicit, bounded, and comparable.

use std::fmt;

/// Failures of the staged-experiment scaffolding.
///
/// Every variant carries the smallest context needed to diagnose the
/// failure: which field, what bound, what was offered. No variant wraps an
/// opaque source error; callers that need I/O detail get a bounded reason
/// string instead, keeping the enum [`Clone`], [`PartialEq`], and [`Eq`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExperimentError {
    /// A required text field was empty.
    EmptyField {
        /// Which field was empty.
        field: &'static str,
    },
    /// A text field exceeded its character bound.
    TextTooLong {
        /// Which field was too long.
        field: &'static str,
        /// The bound in characters.
        max: usize,
        /// The offered length in characters.
        got: usize,
    },
    /// A list field exceeded its item bound.
    TooManyItems {
        /// Which list overflowed.
        field: &'static str,
        /// The bound.
        max: usize,
    },
    /// An identifier failed shape validation.
    BadId {
        /// Which identifier.
        field: &'static str,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// A child capability set was not a *strict* subset of its parent.
    NotStrictSubset,
    /// A child capability set exceeded its parent's tools, paths, or budget.
    CapabilityEscalation {
        /// Which dimension exceeded the parent (static text only).
        detail: &'static str,
    },
    /// A numeric budget that must be positive was zero.
    InvalidBudget {
        /// Which budget.
        field: &'static str,
    },
    /// The scheduler queue is at capacity; the node was rejected, not dropped.
    QueueFull {
        /// The queue capacity in nodes.
        capacity: usize,
    },
    /// A node id was admitted twice.
    DuplicateNode {
        /// The offending node id.
        id: String,
    },
    /// No admitted node has this id.
    UnknownNode {
        /// The requested node id.
        id: String,
    },
    /// A result arrived with a generation that does not match the node.
    StaleGeneration {
        /// The node id.
        node: String,
        /// The generation the scheduler expects.
        expected: u64,
        /// The generation the result carried.
        got: u64,
    },
    /// A result was published twice for the same node (at-most-once).
    DuplicateResult {
        /// The node id.
        node: String,
    },
    /// The run was cancelled; late results are rejected.
    RunCancelled {
        /// The cancelled run id.
        run: String,
    },
    /// A node's generation exceeds the scheduler's delegation depth.
    DepthExceeded {
        /// The configured maximum depth.
        max: u64,
    },
    /// A result publication named a non-terminal state.
    NotTerminal {
        /// The offered state name.
        state: &'static str,
    },
    /// The requested state transition is not allowed from the current state.
    BadTransition {
        /// Where the lifecycle is.
        from: &'static str,
        /// The event the caller attempted.
        event: &'static str,
    },
    /// The lifecycle is in a terminal state and rejects every event.
    LifecycleTerminal {
        /// The terminal state name.
        state: &'static str,
    },
    /// An operator approval record failed shape validation.
    ApprovalRejected {
        /// Why the record was rejected (static text only).
        reason: &'static str,
    },
    /// The evidence bundle is missing a required piece.
    IncompleteEvidence {
        /// What is missing (static text only).
        missing: &'static str,
    },
    /// A budget was exhausted; the operation fails closed.
    BudgetExhausted {
        /// Which budget (static text only).
        what: &'static str,
    },
    /// The absolute deadline passed; the operation fails closed.
    DeadlineExceeded,
    /// A proposal touches a path candidates must never control.
    ProtectedSurface {
        /// The offending path.
        path: String,
    },
    /// A changed-surface path is absolute, empty, or escapes its root.
    BadPath {
        /// The offending path.
        path: String,
    },
    /// A reviewer did not approve; promotion is blocked.
    ReviewerRejected {
        /// The rejecting reviewer's role name.
        reviewer: &'static str,
    },
    /// A manifest failed validation; names the file, key, and reason.
    ManifestInvalid {
        /// Source label, e.g. `"manifests/suites.toml"`.
        file: &'static str,
        /// The offending key or section.
        key: String,
        /// Why it was rejected.
        reason: String,
    },
    /// A manifest file could not be read.
    ManifestUnreadable {
        /// Source label.
        file: &'static str,
        /// Bounded reason string.
        reason: String,
    },
    /// An evaluator stage ran out of order.
    BadStageOrder {
        /// The stage that was required.
        expected: &'static str,
        /// The stage that was attempted.
        got: &'static str,
    },
    /// Record serialization failed (should not happen for our own types).
    SerializationFailed {
        /// Bounded reason string.
        reason: String,
    },
}

impl fmt::Display for ExperimentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField { field } => write!(formatter, "{field} must not be empty"),
            Self::TextTooLong { field, max, got } => {
                write!(formatter, "{field} is {got} chars; the bound is {max}")
            }
            Self::TooManyItems { field, max } => {
                write!(formatter, "{field} holds more than {max} items")
            }
            Self::BadId { field, reason } => {
                write!(formatter, "{field} is not a valid id: {reason}")
            }
            Self::NotStrictSubset => write!(
                formatter,
                "child capabilities must be a strict subset of the parent"
            ),
            Self::CapabilityEscalation { detail } => {
                write!(formatter, "capability escalation denied: {detail}")
            }
            Self::InvalidBudget { field } => {
                write!(formatter, "{field} must be positive")
            }
            Self::QueueFull { capacity } => {
                write!(formatter, "scheduler queue is full at {capacity} nodes; node rejected")
            }
            Self::DuplicateNode { id } => write!(formatter, "node {id} was already admitted"),
            Self::UnknownNode { id } => write!(formatter, "no admitted node {id}"),
            Self::StaleGeneration {
                node,
                expected,
                got,
            } => write!(
                formatter,
                "node {node} result has generation {got}; expected {expected}"
            ),
            Self::DuplicateResult { node } => {
                write!(formatter, "node {node} already published a result")
            }
            Self::RunCancelled { run } => {
                write!(formatter, "run {run} was cancelled; late result rejected")
            }
            Self::DepthExceeded { max } => {
                write!(formatter, "delegation depth exceeds the maximum of {max}")
            }
            Self::NotTerminal { state } => {
                write!(formatter, "{state} is not a terminal state")
            }
            Self::BadTransition { from, event } => {
                write!(formatter, "cannot apply event {event} from state {from}")
            }
            Self::LifecycleTerminal { state } => {
                write!(formatter, "lifecycle is terminal in {state}; all events rejected")
            }
            Self::ApprovalRejected { reason } => {
                write!(formatter, "operator approval record rejected: {reason}")
            }
            Self::IncompleteEvidence { missing } => {
                write!(formatter, "evidence incomplete: {missing}")
            }
            Self::BudgetExhausted { what } => {
                write!(formatter, "{what} budget exhausted; failing closed")
            }
            Self::DeadlineExceeded => write!(formatter, "deadline exceeded; failing closed"),
            Self::ProtectedSurface { path } => write!(
                formatter,
                "proposal touches protected surface {path}; denied"
            ),
            Self::BadPath { path } => {
                write!(formatter, "changed-surface path {path} is not allowed")
            }
            Self::ReviewerRejected { reviewer } => {
                write!(formatter, "reviewer {reviewer} did not approve; promotion blocked")
            }
            Self::ManifestInvalid { file, key, reason } => {
                write!(formatter, "manifest {file}: key {key}: {reason}")
            }
            Self::ManifestUnreadable { file, reason } => {
                write!(formatter, "manifest {file} unreadable: {reason}")
            }
            Self::BadStageOrder { expected, got } => write!(
                formatter,
                "evaluator stage out of order: expected {expected}, got {got}"
            ),
            Self::SerializationFailed { reason } => {
                write!(formatter, "record serialization failed: {reason}")
            }
        }
    }
}

impl std::error::Error for ExperimentError {}

/// Builds [`ExperimentError::ManifestInvalid`] with owned strings.
pub(crate) fn manifest_invalid(
    file: &'static str,
    key: &str,
    reason: &str,
) -> ExperimentError {
    ExperimentError::ManifestInvalid {
        file,
        key: key.to_string(),
        reason: reason.to_string(),
    }
}

/// Builds [`ExperimentError::ManifestUnreadable`] with an owned reason.
pub(crate) fn manifest_unreadable(file: &'static str, reason: &str) -> ExperimentError {
    ExperimentError::ManifestUnreadable {
        file,
        reason: reason.to_string(),
    }
}
