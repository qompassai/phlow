//! Typed failures at the attention protocol boundary.
//!
//! Every variant is a rejection of external data (a snapshot input, a
//! focus advertisement, a board publish) — never a panic path. Callers
//! at coordination gates map these to fail-closed outcomes.

use std::fmt;

/// One way attention-protocol data can be refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionError {
    /// Agent identifier is empty or longer than the accepted bound.
    InvalidAgentId,
    /// Target identifier is empty, over-long, or carries control
    /// characters that would corrupt the canonical snapshot frame.
    InvalidTargetId(String),
    /// The same target appeared twice in one snapshot input.
    DuplicateTarget(String),
    /// A snapshot or focus exceeded its target-count bound.
    TooManyTargets { count: usize, max: usize },
    /// Two foci were compared across different snapshot hashes.
    /// Alignment is undefined across frames; this is the fail-closed
    /// core of the protocol.
    SnapshotMismatch,
    /// A board publish named a different agent than the authenticated
    /// transport caller. Identity comes from the session, never from
    /// the payload.
    IdentityMismatch,
    /// A board publish carried a sequence number that does not advance
    /// the agent's previous one: a stale or replayed focus.
    StaleSequence { agent: String, got: u64, last: u64 },
    /// The board already tracks the maximum number of agents.
    BoardFull,
    /// A focus failed structural validation (see `Focus::validate`).
    InvalidFocus(String),
}

impl fmt::Display for AttentionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttentionError::InvalidAgentId => {
                write!(f, "agent id is empty or over the length bound")
            }
            AttentionError::InvalidTargetId(id) => {
                write!(f, "invalid target id: {id:?}")
            }
            AttentionError::DuplicateTarget(id) => {
                write!(f, "duplicate target in snapshot input: {id}")
            }
            AttentionError::TooManyTargets { count, max } => {
                write!(f, "target count {count} exceeds bound {max}")
            }
            AttentionError::SnapshotMismatch => {
                write!(f, "foci were computed from different snapshots")
            }
            AttentionError::IdentityMismatch => {
                write!(f, "focus agent id does not match the transport caller")
            }
            AttentionError::StaleSequence { agent, got, last } => {
                write!(
                    f,
                    "stale focus sequence for {agent}: got {got}, last accepted {last}"
                )
            }
            AttentionError::BoardFull => {
                write!(f, "focus board already tracks the maximum number of agents")
            }
            AttentionError::InvalidFocus(why) => {
                write!(f, "invalid focus: {why}")
            }
        }
    }
}

impl std::error::Error for AttentionError {}
