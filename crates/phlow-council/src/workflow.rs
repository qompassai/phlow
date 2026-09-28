//! The workflow: the state machine that moves candidates forward.

use crate::candidate::{Candidate, CandidateId, CandidateStatus, SUMMARY_CHARS_MAX};
use crate::contract::check_len;
use crate::error::CouncilError;
use crate::evidence::{Evidence, EvidenceKind};
use crate::review::{CouncilReview, Decision};

/// Largest number of candidates one workflow may hold.
pub const CANDIDATES_MAX: usize = 32;

/// The candidate workflow for one task contract.
///
/// The contract is stored for reference; the workflow tracks candidates,
/// their evidence, and the decisions applied to them.
#[derive(Debug)]
pub struct Workflow {
    contract: crate::contract::TaskContract,
    candidates: Vec<Candidate>,
    evidence: Vec<Evidence>,
    next_id: u64,
}

impl Workflow {
    /// Starts a workflow for `contract`.
    pub fn new(contract: crate::contract::TaskContract) -> Self {
        Self {
            contract,
            candidates: Vec::new(),
            evidence: Vec::new(),
            next_id: 0,
        }
    }

    /// Returns the task contract.
    pub fn contract(&self) -> &crate::contract::TaskContract {
        &self.contract
    }

    /// Proposes a fresh candidate: `Proposed`, no parent.
    pub fn propose(&mut self, summary: &str) -> Result<CandidateId, CouncilError> {
        self.propose_inner(None, summary)
    }

    /// Proposes a revision of `parent`: `Proposed`, with lineage recorded.
    pub fn propose_child(
        &mut self,
        parent: CandidateId,
        summary: &str,
    ) -> Result<CandidateId, CouncilError> {
        self.find(parent)?;
        self.propose_inner(Some(parent), summary)
    }

    /// Marks a proposed candidate implemented: `Proposed -> Implemented`.
    pub fn implement(&mut self, id: CandidateId) -> Result<(), CouncilError> {
        let candidate = self.find_mut(id)?;
        match candidate.status {
            CandidateStatus::Proposed => {
                candidate.status = CandidateStatus::Implemented;
                Ok(())
            }
            from => Err(CouncilError::BadTransition {
                id: id.0,
                from,
                attempted: "implement",
            }),
        }
    }

    /// Attaches evidence to a candidate.
    ///
    /// Passing correctness evidence for an implemented candidate moves it
    /// to `Verified`. All other evidence is retained without moving the
    /// candidate; evidence on terminal candidates is rejected.
    pub fn attach_evidence(&mut self, evidence: Evidence) -> Result<(), CouncilError> {
        let candidate = self.find_mut(evidence.candidate)?;
        match candidate.status {
            CandidateStatus::Rejected | CandidateStatus::Promoted => {
                return Err(CouncilError::BadTransition {
                    id: evidence.candidate.0,
                    from: candidate.status,
                    attempted: "attach evidence to",
                });
            }
            CandidateStatus::Implemented
                if evidence.kind == EvidenceKind::Correctness && evidence.passed =>
            {
                candidate.status = CandidateStatus::Verified;
            }
            _ => {}
        }
        self.evidence.push(evidence);
        Ok(())
    }

    /// Runs one council review round and applies its decision.
    ///
    /// - `Keep` on a verified candidate promotes it; on any other status
    ///   the decision is recorded but the candidate does not move.
    /// - `Revise` returns the candidate to `Proposed`.
    /// - `Reject` is terminal.
    pub fn review(&mut self, review: CouncilReview) -> Result<Decision, CouncilError> {
        let candidate = self.find_mut(review.candidate)?;
        match candidate.status {
            CandidateStatus::Rejected | CandidateStatus::Promoted => {
                return Err(CouncilError::BadTransition {
                    id: review.candidate.0,
                    from: candidate.status,
                    attempted: "review",
                });
            }
            _ => {}
        }
        let decision = review.decision();
        match decision {
            Decision::Keep => {
                if candidate.status == CandidateStatus::Verified {
                    candidate.status = CandidateStatus::Promoted;
                }
            }
            Decision::Revise => {
                candidate.status = CandidateStatus::Proposed;
            }
            Decision::Reject => {
                candidate.status = CandidateStatus::Rejected;
            }
        }
        Ok(decision)
    }

    /// Reports a candidate's status.
    pub fn status(&self, id: CandidateId) -> Result<CandidateStatus, CouncilError> {
        Ok(self.find(id)?.status)
    }

    /// Returns the evidence attached to a candidate, in attach order.
    pub fn evidence_for(&self, id: CandidateId) -> Vec<&Evidence> {
        self.evidence
            .iter()
            .filter(|evidence| evidence.candidate == id)
            .collect()
    }

    /// Returns the candidate's parent, if it revises another.
    pub fn parent_of(&self, id: CandidateId) -> Result<Option<CandidateId>, CouncilError> {
        Ok(self.find(id)?.parent)
    }

    fn propose_inner(
        &mut self,
        parent: Option<CandidateId>,
        summary: &str,
    ) -> Result<CandidateId, CouncilError> {
        if self.candidates.len() >= CANDIDATES_MAX {
            return Err(CouncilError::CandidateCapReached);
        }
        check_len("candidate summary", summary, SUMMARY_CHARS_MAX)?;
        let id = CandidateId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.candidates.push(Candidate {
            id,
            summary: summary.to_string(),
            parent,
            status: CandidateStatus::Proposed,
        });
        Ok(id)
    }

    fn find(&self, id: CandidateId) -> Result<&Candidate, CouncilError> {
        self.candidates
            .iter()
            .find(|candidate| candidate.id == id)
            .ok_or(CouncilError::UnknownCandidate { id: id.0 })
    }

    fn find_mut(&mut self, id: CandidateId) -> Result<&mut Candidate, CouncilError> {
        self.candidates
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .ok_or(CouncilError::UnknownCandidate { id: id.0 })
    }
}
