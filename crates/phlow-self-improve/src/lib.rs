//! Self-improvement surfaces: the SQLite feedback store, the
//! append-only per-run outcome record ([`outcome`], PH-1), the
//! disabled prompt evolver, and the read-only skill store.
//!
//! The proposal surfaces ([`budget`], PH-2, and [`reviewer_cal`],
//! PH-5) measure recorded outcomes and render proposals for the
//! operator; they read the outcome store read-only and apply
//! nothing.
//!
//! Nothing here mutates prompts, skills, or Git history automatically.
//! Feedback is recorded for operator review; evolution and mutation always
//! fail with the exact Python denials.
//!
//! # Limits
//!
//! - Feedback text fields: [`feedback::SESSION_ID_CHARS_MAX`],
//!   [`feedback::COMMENT_CHARS_MAX`], [`feedback::PROMPT_USED_CHARS_MAX`],
//!   [`feedback::OUTCOME_CHARS_MAX`] characters.
//! - Low-rated listing: [`feedback::LOW_RATED_LIMIT_MAX`] rows.
//! - Skill names: [`skill_store::SKILL_NAME_CHARS_MAX`] characters;
//!   listings: the workspace's bounded recursive file list (at most
//!   `phlow_workspace::LIST_FILES_MAX` files, as in Python).

#![forbid(unsafe_code)]

pub mod budget;
pub mod error;
pub mod feedback;
pub mod outcome;
pub mod prompt_evolver;
pub mod reviewer_cal;
pub mod skill_store;

pub use budget::{
    BudgetProposal, ClassFit, CorpusLabel, LabelledRun, Margin, OutcomeRow, ReplayReport, Split,
    Truth,
};
pub use error::SelfImproveError;
pub use feedback::{FeedbackEntry, FeedbackStore};
pub use outcome::{OutcomeStore, RecordedOutcome, RunOutcome, VerificationStrength};
pub use prompt_evolver::PromptEvolver;
pub use reviewer_cal::{CalibrationVerdict, ReliabilityRow, Verdict};
pub use skill_store::SkillStore;
