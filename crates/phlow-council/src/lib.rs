#![forbid(unsafe_code)]

//! A small candidate workflow: propose, implement, verify, review, iterate.
//!
//! The methodology is adapted from NVlabs kda's agent flow — a task
//! contract, one implemented candidate at a time, retained evidence,
//! and a keep/revise/reject council decision per iteration. Everything
//! here is original code and prose; no kda text, prompts, or skills are
//! reproduced.
//!
//! The workflow is a bounded state machine:
//!
//! ```text
//! Proposed --implement--> Implemented --verify--> Verified --review(keep)--> Promoted
//!     ^                       |                       |
//!     +---- review(revise) ----+---- review(revise) ----+
//!     review(reject) --> Rejected (terminal)
//! ```
//!
//! Every transition is typed; misuse returns [`CouncilError`] instead of
//! panicking.

mod candidate;
mod contract;
mod error;
mod evidence;
mod review;
mod workflow;

pub use candidate::{Candidate, CandidateId, CandidateStatus};
pub use contract::TaskContract;
pub use error::CouncilError;
pub use evidence::{Evidence, EvidenceKind};
pub use review::{CouncilReview, Decision, Vote};
pub use workflow::{CANDIDATES_MAX, Workflow};
