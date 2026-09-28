//! Candidates: one proposed solution at a time, with lineage.

/// Largest number of characters in a candidate summary.
pub const SUMMARY_CHARS_MAX: usize = 1024;

/// A candidate's id. Issued by [`Workflow`][crate::Workflow], increasing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CandidateId(pub u64);

/// Where a candidate is in the workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateStatus {
    /// Proposed but not yet built.
    Proposed,
    /// Built; awaiting verification evidence.
    Implemented,
    /// Correctness evidence passed; awaiting council review.
    Verified,
    /// Rejected by council review; terminal.
    Rejected,
    /// Kept by council review after verification; terminal.
    Promoted,
}

/// One candidate solution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The workflow-issued id.
    pub id: CandidateId,
    /// What this candidate tries, in bounded prose.
    pub summary: String,
    /// The candidate this one revises, if any.
    pub parent: Option<CandidateId>,
    /// The current workflow status.
    pub status: CandidateStatus,
}
