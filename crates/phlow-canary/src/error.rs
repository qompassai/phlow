//! The single typed error for every canary battery failure.
//!
//! Every variant carries bounded, fixed-shape context. None ever carries
//! payload text: payloads are canary content and must not leak into
//! errors, reports or logs (design: "Canary content is not published").

use std::fmt;

/// Why a canary operation produced no usable result.
///
/// Callers must treat every variant as "the battery did not pass":
/// the verdict layer is fail-closed, so an error can only ever refuse
/// a model, never deploy one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanaryError {
    /// The payload store file is missing, unreadable or not a file.
    PayloadStoreIo { reason: String },
    /// The payload store grants group/other access; canary content must
    /// be owner-only (unix mode with no `0o077` bits set).
    PayloadStorePermissions { mode: u32 },
    /// The payload store parsed but violated a named limit or shape rule.
    PayloadStoreInvalid { reason: &'static str },
    /// The threshold book is missing, unreadable or malformed.
    ThresholdBookIo { reason: String },
    /// A threshold value is non-finite or outside its sane range.
    ThresholdInvalid { reason: &'static str },
    /// No calibrated thresholds exist for this model. Per the cross-LLM
    /// generalization findings in the design, thresholds are per-model;
    /// an uncalibrated model is refused, never judged by global numbers.
    UncalibratedModel { model_id: String },
    /// A file write (report, split log, verdict cache) failed.
    Io { reason: String },
    /// The model artifact exceeded the named hashing bound.
    ModelTooLarge { bytes_max: u64 },
}

impl fmt::Display for CanaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanaryError::PayloadStoreIo { reason } => {
                write!(f, "canary payload store: {reason}")
            }
            CanaryError::PayloadStorePermissions { mode } => write!(
                f,
                "canary payload store: permissions {mode:o} grant group/other access"
            ),
            CanaryError::PayloadStoreInvalid { reason } => {
                write!(f, "canary payload store invalid: {reason}")
            }
            CanaryError::ThresholdBookIo { reason } => {
                write!(f, "canary threshold book: {reason}")
            }
            CanaryError::ThresholdInvalid { reason } => {
                write!(f, "canary threshold invalid: {reason}")
            }
            CanaryError::UncalibratedModel { model_id } => {
                write!(f, "canary: no calibrated thresholds for model {model_id}")
            }
            CanaryError::Io { reason } => write!(f, "canary io: {reason}"),
            CanaryError::ModelTooLarge { bytes_max } => {
                write!(f, "canary: model artifact exceeds {bytes_max} bytes")
            }
        }
    }
}

impl std::error::Error for CanaryError {}
