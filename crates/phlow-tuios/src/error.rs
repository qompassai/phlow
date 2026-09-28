//! Error type for the crate, with the stable wire codes the control protocol
//! reports. The codes are the public surface: a client matches on the code
//! string, never on message text.

use std::fmt;

/// Stable string error codes for the control protocol, adapted from tuios's
/// verb error catalog (`internal/session/verb_protocol.go`, the `ErrVerb*`
/// constants). Only the codes this crate's verbs can raise are carried over;
/// tuios-only codes (pty_not_found, needs_client, ...) are not invented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The request line was not a valid envelope (bad JSON, missing verb).
    InvalidRequest,
    /// No verb by that name is registered.
    UnknownVerb,
    /// Params failed to decode, a required field was missing, or an unknown
    /// parameter name was supplied (unknown names are refused, never ignored).
    InvalidParams,
    /// The named session does not exist.
    SessionNotFound,
    /// `new-session` was given a name the daemon already holds.
    SessionExists,
    /// The window target did not resolve.
    WindowNotFound,
    /// The target agent was mid-turn, so a write was refused without waiting.
    NotReady,
    /// An ask addressed an agent on `needs_input`: somebody has to read the
    /// prompt and answer it; waiting does not clear this.
    AgentBlocked,
    /// The call would loop: a pane addressing itself, or an ask closing a
    /// cycle with one already in flight. The remedy is to restructure.
    LoopRefused,
    /// The sender is over the message rate cap. The remedy is to wait.
    RateLimited,
    /// The caller may not do what it asked (e.g. a pane claiming to be human).
    Forbidden,
    /// A wait-for condition did not match before its timeout.
    Timeout,
    /// Unexpected server-side failure.
    Internal,
}

impl ErrorCode {
    /// The wire spelling. These strings are stable; do not rename.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::UnknownVerb => "unknown_verb",
            Self::InvalidParams => "invalid_params",
            Self::SessionNotFound => "session_not_found",
            Self::SessionExists => "session_exists",
            Self::WindowNotFound => "window_not_found",
            Self::NotReady => "not_ready",
            Self::AgentBlocked => "agent_blocked",
            Self::LoopRefused => "loop_refused",
            Self::RateLimited => "rate_limited",
            Self::Forbidden => "forbidden",
            Self::Timeout => "timeout",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The crate's error type. Expected failures (bad input, missing names,
/// refused calls) are values here; corrupt internal relationships are
/// `assert!`s at the call site, not variants here.
#[derive(Debug)]
pub enum TuiosError {
    /// A protocol-level failure with a stable wire code and a bounded message.
    Protocol { code: ErrorCode, message: String },
    /// Local I/O or socket failure, never sent on the wire as-is.
    Io(std::io::Error),
    /// A request frame exceeded [`crate::protocol::FRAME_BYTES_MAX`].
    FrameTooLarge { bytes: u64 },
}

impl TuiosError {
    /// Build a protocol error. The message is truncated to a bound so a
    /// hostile peer cannot make the daemon echo unbounded text.
    pub fn protocol(code: ErrorCode, message: impl Into<String>) -> Self {
        let message: String = message.into();
        // Bound: error text a client can provoke, in bytes.
        const MESSAGE_BYTES_MAX: usize = 1024;
        let message = crate::truncate_bytes(&message, MESSAGE_BYTES_MAX);
        Self::Protocol { code, message }
    }

    /// The wire code, or `Internal` for non-protocol failures.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Protocol { code, .. } => *code,
            Self::Io(_) | Self::FrameTooLarge { .. } => ErrorCode::Internal,
        }
    }
}

impl fmt::Display for TuiosError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol { code, message } => write!(f, "{code}: {message}"),
            Self::Io(err) => write!(f, "io: {err}"),
            Self::FrameTooLarge { bytes } => write!(f, "frame too large: {bytes} bytes"),
        }
    }
}

impl std::error::Error for TuiosError {}

impl From<std::io::Error> for TuiosError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validation ---

    #[test]
    fn wire_codes_are_stable_strings() {
        assert_eq!(ErrorCode::InvalidRequest.as_str(), "invalid_request");
        assert_eq!(ErrorCode::UnknownVerb.as_str(), "unknown_verb");
        assert_eq!(ErrorCode::LoopRefused.as_str(), "loop_refused");
        assert_eq!(ErrorCode::RateLimited.as_str(), "rate_limited");
    }

    // --- adversarial ---

    #[test]
    fn protocol_message_is_truncated_to_bound() {
        let err = TuiosError::protocol(ErrorCode::Internal, "x".repeat(5000));
        let TuiosError::Protocol { message, .. } = &err else {
            panic!("expected protocol error");
        };
        assert_eq!(message.len(), 1024);
    }
}
