#![forbid(unsafe_code)]

//! Staged supervised-self-improvement experiment scaffolding.
//!
//! # Status: EXPERIMENTAL
//!
//! This crate is scaffolding for the staged experiment described in
//! `docs/experiments/PHLOW_EXPERIMENTAL_SELF_IMPROVEMENT.md`. It defines
//! types, manifests, fixtures, and pure-logic invariants — it enables no
//! runtime concurrency, no scheduler execution, and no self-editing
//! behavior. Nothing here can modify Phlow, approve a candidate, or weaken
//! a gate on its own.
//!
//! # Hard constraints (enforced by types, not by convention)
//!
//! - **No autonomous self-modification.** [`promotion::ImprovementProposal`]
//!   is inert data; only an explicit [`promotion::HumanApproval`] plus
//!   complete evidence can pass [`promotion::PromotionGate`].
//! - **No self-approval or self-promotion.**
//!   [`promotion::HumanApproval`] is opaque and constructible only from a
//!   shape-validated operator record; model output cannot become one.
//! - **Promotion gates stay human.**
//!   [`promotion::Lifecycle`] requires the `HumanApproved` event to leave
//!   `AwaitingHuman`; terminal states reject every event.
//! - **Read-only default.** [`control_plane::WorkerRole::can_write_production`]
//!   is false for every role, asserted rather than configured.
//! - **Fail closed.** Missing evidence, passed deadlines, exhausted
//!   budgets, stale generations, and incomplete records are typed errors or
//!   explicit `false` — never inferred success.
//!
//! # Layout
//!
//! - [`control_plane`]: scheduler types and pure scheduling invariants
//!   (states, roles, capability delegation, bounded admission, at-most-once
//!   publication, cancellation).
//! - [`promotion`]: candidate lifecycle, opaque human approval,
//!   improvement proposals, and the promotion gate.
//! - [`evaluator`]: evaluation stages, budget tracking, and evidence
//!   bundles (skeleton: order and budgets, no execution).
//! - [`record`]: the immutable per-experiment JSON evaluation report.
//! - [`manifest`]: parsing and validation for the TOML contracts in
//!   `manifests/` and `evals/`.
//! - [`error`]: the single typed error for the whole crate.

mod control_plane;
mod error;
mod evaluator;
mod manifest;
mod promotion;
mod record;

pub use control_plane::{
    AGGREGATE_OUTPUT_BYTES_MAX_DEFAULT, AGGREGATE_TOOL_CALLS_MAX_DEFAULT,
    CANCELLED_RUNS_MAX, CAPABILITY_ENTRY_CHARS_MAX, CAPABILITY_PATHS_MAX,
    CAPABILITY_TOOLS_MAX, CHILDREN_PER_TASK_MAX_DEFAULT, DEPENDENCY_IDS_MAX,
    DEPTH_MAX_DEFAULT, DIGEST_CHARS_MAX, ID_CHARS_MAX, QUEUE_CAPACITY_DEFAULT,
    REVISION_CHARS_MAX, TASK_DEADLINE_MS_DEFAULT, WORKERS_MAX_DEFAULT, CapabilitySet,
    ExperimentId, NodeId, NodeParams, NodeState, RunId, Scheduler, SchedulerLimits,
    SchedulerNode, WorkerRole, WriteAccess,
};
pub use error::ExperimentError;
pub use evaluator::{
    ARG_ENTRY_CHARS_MAX, ARTIFACTS_MAX, CHECK_ARGV_MAX, CHECKS_MAX, COVERAGE_MAX,
    EVIDENCE_NAME_CHARS_MAX, STAGE_TRANSITIONS_MAX, ArtifactDigest, BudgetTracker,
    CheckRun, EvalStage, Evaluator, EvidenceBundle, VerificationOutcome,
};
pub use manifest::{
    FORBIDDEN_PATHS_MAX, LANGUAGES_PER_TIER_MAX, MANIFEST_DESC_CHARS_MAX,
    MANIFEST_NAME_CHARS_MAX, MANIFEST_SCHEMA_VERSION, REQUIRED_FILES_MAX, SUITES_MAX,
    TASK_CHECKS_MAX, TIERS_MAX, Acceptance, BudgetDefaults, BudgetManifest,
    LanguageDef, LanguageManifest, PromotionManifest, PromotionThresholds, RiskClass,
    SuiteDef, SuiteManifest, TaskBudget, TaskCheck, TaskManifest, TierDef,
    parse_budget_manifest, parse_language_manifest, parse_promotion_manifest,
    parse_suite_manifest, parse_task_manifest, read_manifest_file,
};
pub use promotion::{
    APPROVAL_FIELD_CHARS_MAX, APPROVAL_SCOPE_CHARS_MAX, CANDIDATE_DIGEST_HEX_MIN,
    CHANGED_SURFACE_MAX, DIFF_SUMMARY_CHARS_MAX, DIGEST_HEX_CHARS_MAX,
    OPERATOR_RECORD_CHARS_MAX, PROPOSAL_TEXT_CHARS_MAX, REVIEWER_DECISIONS_MAX,
    SURFACE_PATH_CHARS_MAX, HumanApproval, ImprovementProposal, Lifecycle,
    LifecycleEvent, PromotionGate, PromotionRecord, ProposalBudgets, ProposalParams,
    ReviewDecision, ReviewerDecision, check_proposal_surface,
};
pub use record::{
    CHANGED_FILES_MAX, EVENTS_MAX, RECORD_CHECKS_MAX, RECORD_LIST_MAX,
    RECORD_TEXT_CHARS_MAX, SCHEMA_VERSION, CheckRecord, EvaluationRecord,
    PromotionSection, RecordParams, VerificationSection,
};
