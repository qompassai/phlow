//! Council review: bounded reviewers vote keep, revise, or reject.

use crate::candidate::CandidateId;
use crate::contract::check_len;
use crate::error::CouncilError;

/// Largest number of reviewers on one council.
pub const REVIEWERS_MAX: usize = 8;
/// Largest number of characters in a reviewer name.
pub const REVIEWER_CHARS_MAX: usize = 64;

/// One reviewer's vote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vote {
    /// The candidate is good; promote it if verified.
    Keep,
    /// The candidate needs rework; send it back to proposed.
    Revise,
    /// The candidate is rejected; terminal.
    Reject,
}

/// The council's decision for one candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Promote the candidate (only from verified).
    Keep,
    /// Return the candidate to proposed for rework.
    Revise,
    /// Reject the candidate; terminal.
    Reject,
}

/// One round of council review: named reviewers, one vote each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CouncilReview {
    /// The candidate under review.
    pub candidate: CandidateId,
    /// Each reviewer's vote.
    pub votes: Vec<(String, Vote)>,
}

impl CouncilReview {
    /// Builds a review, validating reviewer bounds and duplicates.
    ///
    /// # Contract
    /// - Accepts: 1..=[`REVIEWERS_MAX`] reviewers, each named within
    ///   [`REVIEWER_CHARS_MAX`] chars, no duplicate names.
    /// - Rejects: empty councils, oversized councils, overlong or
    ///   duplicated reviewer names — with a typed error.
    pub fn new(candidate: CandidateId, votes: &[(&str, Vote)]) -> Result<Self, CouncilError> {
        if votes.is_empty() || votes.len() > REVIEWERS_MAX {
            return Err(CouncilError::TooManyItems {
                field: "reviewers",
                max: REVIEWERS_MAX,
            });
        }
        let mut owned = Vec::with_capacity(votes.len());
        for (name, vote) in votes {
            check_len("reviewer name", name, REVIEWER_CHARS_MAX)?;
            if owned.iter().any(|(existing, _)| existing == name) {
                return Err(CouncilError::DuplicateReviewer {
                    name: (*name).to_string(),
                });
            }
            owned.push(((*name).to_string(), *vote));
        }
        Ok(Self {
            candidate,
            votes: owned,
        })
    }

    /// Tallies the votes into a decision.
    ///
    /// Keep wins on a strict plurality over both reject and revise; reject
    /// wins on a strict plurality over keep while at least tying revise;
    /// every tie and every revise-plurality returns revise — the safe
    /// default is more work, not a verdict.
    pub fn decision(&self) -> Decision {
        let (mut keep, mut revise, mut reject) = (0usize, 0usize, 0usize);
        for (_, vote) in &self.votes {
            match vote {
                Vote::Keep => keep += 1,
                Vote::Revise => revise += 1,
                Vote::Reject => reject += 1,
            }
        }
        if keep > reject && keep > revise {
            Decision::Keep
        } else if reject > keep && reject >= revise {
            Decision::Reject
        } else {
            Decision::Revise
        }
    }
}
