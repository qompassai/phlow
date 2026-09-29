//! The single typed error for every System 1 failure.

use std::fmt;

use phlow_llm::redact_credentials;

/// Characters of server- or transport-supplied detail kept in an error.
const DETAIL_CHARS_MAX: usize = 256;

/// Why a System 1 call produced no usable answers.
///
/// Every variant carries bounded context only, and none ever carries the
/// API key: config errors use fixed text, and transport detail is
/// credential-redacted and truncated. Callers must treat every variant as
/// "no answer" and fall back to the existing path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum System1Error {
    /// The endpoint, model or key configuration is unusable.
    Config { reason: &'static str },
    /// The question batch violates a named limit or shape rule; nothing was sent.
    InvalidBatch { reason: &'static str },
    /// The request could not be sent or the server answered with a non-2xx status.
    Transport { detail: String },
    /// The request exceeded its deadline.
    Timeout,
    /// The server's response was malformed, oversized or out of contract.
    Protocol { reason: String },
}

impl System1Error {
    /// Build [`System1Error::Transport`] with redacted, truncated detail.
    pub(crate) fn transport(detail: &str) -> Self {
        System1Error::Transport {
            detail: bounded(detail),
        }
    }

    /// Build [`System1Error::Protocol`] with truncated detail.
    pub(crate) fn protocol(reason: &str) -> Self {
        System1Error::Protocol {
            reason: bounded(reason),
        }
    }
}

/// Redact credentials first, then truncate, so a cut can never expose a
/// token prefix that redaction would otherwise have recognized.
fn bounded(text: &str) -> String {
    redact_credentials(text)
        .chars()
        .take(DETAIL_CHARS_MAX)
        .collect()
}

impl fmt::Display for System1Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            System1Error::Config { reason } => write!(f, "system1 config: {reason}"),
            System1Error::InvalidBatch { reason } => write!(f, "system1 batch: {reason}"),
            System1Error::Transport { detail } => write!(f, "system1 transport: {detail}"),
            System1Error::Timeout => write!(f, "system1 request timed out"),
            System1Error::Protocol { reason } => write!(f, "system1 protocol: {reason}"),
        }
    }
}

impl std::error::Error for System1Error {}
