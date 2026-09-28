//! Evidence: retained proof attached to a candidate.

use crate::candidate::CandidateId;
use crate::contract::check_len;
use crate::error::CouncilError;

/// Largest number of characters in an evidence note.
pub const NOTE_CHARS_MAX: usize = 1024;

/// What kind of proof a piece of evidence carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    /// A correctness check ran against the implementation.
    Correctness,
    /// A measurement or profile of the implementation.
    Profile,
    /// A reviewer's written note.
    Review,
}

/// One retained piece of proof for a candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// The candidate this evidence belongs to.
    pub candidate: CandidateId,
    /// What kind of proof it is.
    pub kind: EvidenceKind,
    /// Whether the check passed or the note is favorable.
    pub passed: bool,
    /// Bounded prose describing the evidence.
    pub note: String,
}

impl Evidence {
    /// Builds a piece of evidence, validating the note bound.
    pub fn new(
        candidate: CandidateId,
        kind: EvidenceKind,
        passed: bool,
        note: &str,
    ) -> Result<Self, CouncilError> {
        check_len("evidence note", note, NOTE_CHARS_MAX)?;
        Ok(Self {
            candidate,
            kind,
            passed,
            note: note.to_string(),
        })
    }
}
