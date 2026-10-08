//! The single typed error for the whole crate.

use std::fmt;

/// Failures returned by phlow-trainlab operations.
///
/// Expected failures (bad configuration, unknown splits, sampler or
/// executor failures, confirmation-gate refusals) are values of this
/// type, never panics. Internal invariant violations use `assert!`.
#[derive(Debug)]
pub enum TrainlabError {
    /// Configuration or caller input failed validation.
    InvalidConfig(String),
    /// A named limit (sizes, counts, deadlines) was exceeded.
    LimitExceeded(String),
    /// A split name this crate does not freeze tasks for.
    UnknownSplit(String),
    /// A task-family name this crate does not define.
    UnknownFamily(String),
    /// The sampler (stub or Ollama) failed to produce completions.
    Sampler(String),
    /// The executor could not run a completion at all (spawn failure,
    /// unacknowledged code execution). A completion that *ran* and
    /// failed is an `Invalid` evaluation, not this error.
    Executor(String),
    /// The confirmation gate refused an operation.
    Confirmation(String),
    /// A receipt could not be written or verified.
    Receipt(String),
    /// A groups export could not be written, loaded, or verified
    /// against its run receipt.
    Export(String),
    /// Underlying I/O failure.
    Io(std::io::Error),
    /// Underlying JSON failure.
    Json(serde_json::Error),
}

impl fmt::Display for TrainlabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrainlabError::InvalidConfig(msg) => write!(f, "invalid configuration: {msg}"),
            TrainlabError::LimitExceeded(msg) => write!(f, "limit exceeded: {msg}"),
            TrainlabError::UnknownSplit(name) => write!(f, "unknown split: {name}"),
            TrainlabError::UnknownFamily(name) => write!(f, "unknown task family: {name}"),
            TrainlabError::Sampler(msg) => write!(f, "sampler failure: {msg}"),
            TrainlabError::Executor(msg) => write!(f, "executor failure: {msg}"),
            TrainlabError::Confirmation(msg) => write!(f, "confirmation gate: {msg}"),
            TrainlabError::Receipt(msg) => write!(f, "receipt failure: {msg}"),
            TrainlabError::Export(msg) => write!(f, "groups export failure: {msg}"),
            TrainlabError::Io(err) => write!(f, "i/o failure: {err}"),
            TrainlabError::Json(err) => write!(f, "json failure: {err}"),
        }
    }
}

impl std::error::Error for TrainlabError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TrainlabError::Io(err) => Some(err),
            TrainlabError::Json(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for TrainlabError {
    fn from(err: std::io::Error) -> Self {
        TrainlabError::Io(err)
    }
}

impl From<serde_json::Error> for TrainlabError {
    fn from(err: serde_json::Error) -> Self {
        TrainlabError::Json(err)
    }
}
