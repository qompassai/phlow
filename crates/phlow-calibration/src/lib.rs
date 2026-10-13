//! Confidence calibration for phlow's decision models.
//!
//! This crate is the measurement half of the calibration harness
//! designed on 2026-10-04 (`calibration-harness-design-2026-10-04.md`):
//! it collects [`CalibrationTuple`]s from the three sources the design
//! names (gauntlet decision traces, approval history, the feedback
//! store), computes Brier scores, expected calibration error, and
//! per-question-type calibration curves over them, renders the
//! design's Phase A threshold report, and exports weighted
//! fine-tuning datasets. The fine-tuning itself is an external ML
//! workload and is explicitly not in this crate.
//!
//! # Data honesty
//!
//! Nothing here fabricates a label, a distribution, or a data source:
//!
//! - Gauntlet traces carry `ground_truth: null` until a real label
//!   exists (the design's label-pending state). Unlabeled tuples are
//!   counted and reported, never scored.
//! - A distribution is scored only where one genuinely exists: a
//!   recorded distribution, or a Noul answer's Bernoulli completion
//!   `[1 - p, p]`, which is the protocol's own construction of the
//!   full distribution from its scalar. Choice and Score answers from
//!   scalar backends have no distribution and none is invented.
//! - The feedback store and the consumed-approvals store are read
//!   only when the file already exists; collecting never creates
//!   them. Both are reported as source states, with the reason they
//!   yield no tuples stated in the report (see [`collector`]).
//!
//! # Documented interpretations where the design was silent
//!
//! - `decided_at` is stored as epoch milliseconds (`u64`), the same
//!   lossless choice the gauntlet's trace capture made to stay free
//!   of a chrono dependency (the workspace has none).
//! - A Score prediction counts as correct when its value equals the
//!   true value exactly. The design defines no tolerance band, and
//!   the scenario legends Score questions grade against are
//!   discrete; inventing a band would invent labels.
//! - Score tuples never enter the full-distribution Brier: their
//!   ground truth is a scalar, which cannot be aligned one-hot over
//!   a distribution's levels without inventing a level mapping.
//!
//! # Bounds
//!
//! Every input is bounded before it is read: trace files by
//! [`collector::TRACE_FILE_BYTES_MAX`] and per-line by the gauntlet's
//! own [`phlow_gauntlet::trace::TRACE_LINE_BYTES_MAX`], the approvals
//! store by [`collector::APPROVAL_FILE_BYTES_MAX`], feedback rows by
//! the store's own [`phlow_self_improve::feedback::LOW_RATED_LIMIT_MAX`],
//! and dataset exports by [`dataset::DATASET_ROWS_MAX`].

#![forbid(unsafe_code)]

pub mod collector;
pub mod dataset;
pub mod metrics;
pub mod report;
pub mod tuple;

use std::fmt;
use std::path::PathBuf;

pub use collector::{
    ApprovalCollection, Collection, CollectionPaths, FeedbackCollection, GauntletCollection,
    SourceState, TaskOutcome, collect_all, collect_approval_store, collect_feedback,
    collect_gauntlet_traces,
};
pub use dataset::{DatasetRow, export_rows, write_jsonl};
pub use metrics::{
    BINS_DEFAULT, CalibrationBin, CalibrationCurve, GateVerdict, KindMetrics, Metrics,
    SplitMetrics, WELL_CALIBRATED_BRIER_MAX, WELL_CALIBRATED_ECE_MAX, compute_metrics,
    compute_split_metrics, gate_verdict,
};
pub use report::render_report;
pub use tuple::{CalibrationTuple, GroundTruth, QuestionKind, TupleSource};

/// Why a calibration operation failed. Every variant is fail-closed:
/// malformed input is rejected whole, never partially collected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalibrationError {
    /// A line in the consumed-approvals store is malformed.
    ApprovalStore {
        /// The store file being read.
        path: PathBuf,
        /// 1-based line number of the malformed record.
        line: usize,
        /// What was wrong with the record.
        reason: String,
    },
    /// The feedback store could not be read.
    FeedbackStore {
        /// The store's own error text.
        reason: String,
    },
    /// An input file exceeds its byte bound; it is not read at all.
    FileTooLarge {
        /// The oversized file.
        path: PathBuf,
        /// Its size in bytes.
        bytes: u64,
        /// The bound it exceeded.
        max: u64,
    },
    /// A filesystem read failed.
    Io {
        /// The path being read or written.
        path: PathBuf,
        /// The OS error text.
        reason: String,
    },
    /// One JSONL record line exceeds the per-line byte bound.
    LineTooLong {
        /// The file containing the line.
        path: PathBuf,
        /// 1-based line number.
        line: usize,
        /// The line's length in bytes.
        bytes: usize,
        /// The bound it exceeded.
        max: usize,
    },
    /// A JSONL record line is malformed or fails validation.
    MalformedLine {
        /// The file containing the line.
        path: PathBuf,
        /// 1-based line number.
        line: usize,
        /// What was wrong with the record.
        reason: String,
    },
    /// A trace record carries a `kind` this collector does not know.
    UnknownRecordKind {
        /// The file containing the record.
        path: PathBuf,
        /// 1-based line number.
        line: usize,
        /// The unrecognized `kind` value.
        kind: String,
    },
    /// A tuple or metric input violates the calibration contract.
    Validation {
        /// What was violated.
        reason: String,
    },
}

impl fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CalibrationError::ApprovalStore { path, line, reason } => {
                write!(f, "{} line {line}: {reason}", path.display())
            }
            CalibrationError::FeedbackStore { reason } => {
                write!(f, "feedback store: {reason}")
            }
            CalibrationError::FileTooLarge { path, bytes, max } => {
                write!(
                    f,
                    "{} is {bytes} bytes, over the {max} byte bound",
                    path.display()
                )
            }
            CalibrationError::Io { path, reason } => {
                write!(f, "{}: {reason}", path.display())
            }
            CalibrationError::LineTooLong {
                path,
                line,
                bytes,
                max,
            } => {
                write!(
                    f,
                    "{} line {line} is {bytes} bytes, over the {max} byte bound",
                    path.display()
                )
            }
            CalibrationError::MalformedLine { path, line, reason } => {
                write!(f, "{} line {line}: {reason}", path.display())
            }
            CalibrationError::UnknownRecordKind { path, line, kind } => {
                write!(
                    f,
                    "{} line {line}: unknown record kind '{kind}'",
                    path.display()
                )
            }
            CalibrationError::Validation { reason } => {
                write!(f, "calibration contract violated: {reason}")
            }
        }
    }
}

impl std::error::Error for CalibrationError {}
