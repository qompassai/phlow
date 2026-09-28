//! Typed council errors. Every workflow misuse names what it found.

use crate::candidate::CandidateStatus;
use std::fmt;

/// Failures of the candidate workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CouncilError {
    /// The contract's goal text was empty.
    EmptyGoal,
    /// A text field exceeded its bound.
    TextTooLong {
        /// Which field was too long.
        field: &'static str,
        /// The bound in characters.
        max: usize,
        /// The offered length in characters.
        got: usize,
    },
    /// A list field exceeded its bound.
    TooManyItems {
        /// Which list overflowed.
        field: &'static str,
        /// The bound.
        max: usize,
    },
    /// No candidate has this id.
    UnknownCandidate {
        /// The requested id.
        id: u64,
    },
    /// The requested transition is not allowed from the current status.
    BadTransition {
        /// The candidate.
        id: u64,
        /// Where it is.
        from: CandidateStatus,
        /// Where the caller wanted it to go.
        attempted: &'static str,
    },
    /// The same reviewer voted twice.
    DuplicateReviewer {
        /// The reviewer's name.
        name: String,
    },
    /// The workflow already holds the maximum number of candidates.
    CandidateCapReached,
}

impl fmt::Display for CouncilError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyGoal => write!(formatter, "contract goal must not be empty"),
            Self::TextTooLong { field, max, got } => {
                write!(formatter, "{field} is {got} chars; the bound is {max}")
            }
            Self::TooManyItems { field, max } => {
                write!(formatter, "{field} holds more than {max} items")
            }
            Self::UnknownCandidate { id } => {
                write!(formatter, "no candidate with id {id}")
            }
            Self::BadTransition {
                id,
                from,
                attempted,
            } => write!(
                formatter,
                "candidate {id} is {from:?}; cannot {attempted} from there"
            ),
            Self::DuplicateReviewer { name } => {
                write!(formatter, "reviewer {name} voted twice")
            }
            Self::CandidateCapReached => {
                write!(formatter, "candidate cap reached")
            }
        }
    }
}

impl std::error::Error for CouncilError {}
