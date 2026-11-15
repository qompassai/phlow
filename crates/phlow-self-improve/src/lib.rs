//! Self-improvement surfaces: the SQLite feedback store, the
//! append-only per-run outcome record ([`outcome`], PH-1), the
//! disabled prompt evolver, and the read-only skill store.
//!
//! The proposal surfaces ([`budget`], PH-2, and [`reviewer_cal`],
//! PH-5) measure recorded outcomes and render proposals for the
//! operator; they read the outcome store read-only and apply
//! nothing.
//!
//! PH-6 adds the plan-divergence record ([`divergence`]) and the
//! lesson cards ([`lessons`]) built from it: append-only records,
//! versioned cards, retrieved by the runtime as planner context —
//! never as prompt mutation.
//!
//! PH-3 adds escapes and check proposals ([`escape`]): defects the
//! approved checks passed, recorded append-only and clustered into
//! proposed checks that are validated against both revisions and
//! queued through `phlow-approval` — proposed, never admitted.
//!
//! PH-7 adds lesson capture ([`capture`]): a finished run becomes a
//! labelled lesson record for bloch, with the lane derived from
//! flowguard's own trajectory label inside the capture — never
//! caller-asserted, never copied from the outcome row.
//!
//! PH-8 adds skill trust from run rows ([`skill_trust`]): outcome
//! rows record the skills a run loaded, and per-skill standing is
//! recomputed from those rows — evidence only, nothing applied.
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
pub mod capture;
pub mod divergence;
pub mod error;
pub mod escape;
pub mod feedback;
pub mod lessons;
pub mod outcome;
pub mod prompt_evolver;
pub mod reviewer_cal;
pub mod skill_store;
pub mod skill_trust;

pub use budget::{
    BudgetProposal, ClassFit, CorpusLabel, LabelledRun, Margin, OutcomeRow, ReplayReport, Split,
    Truth,
};
pub use capture::{
    CaptureStore, LessonCapture, RecordedCapture, lane_labels_for_trajectory,
};
pub use divergence::{DivergenceStore, PlanDivergence, RecordedDivergence, derive_divergence};
pub use error::SelfImproveError;
pub use escape::{
    CheckValidation, Escape, EscapeCluster, EscapeStore, ProposalSection, RecordedEscape,
    cluster_escapes, queue_check_proposal, render_check_proposals, validate_check,
};
pub use feedback::{FeedbackEntry, FeedbackStore};
pub use lessons::{LessonCard, LessonDivergence, LessonStore, render_card};
pub use outcome::{OutcomeStore, RecordedOutcome, RunOutcome, VerificationStrength};
pub use prompt_evolver::PromptEvolver;
pub use reviewer_cal::{
    CalibrationVerdict, ReliabilityRow, Verdict, VerdictChange, rederive_final_verdicts,
};
pub use skill_store::SkillStore;
pub use skill_trust::{SkillEvidence, SkillState, SkillTrustRow};
