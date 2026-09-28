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
    AGGREGATE_OUTPUT_BYTES_MAX_DEFAULT, AGGREGATE_TOOL_CALLS_MAX_DEFAULT, CANCELLED_RUNS_MAX,
    CAPABILITY_ENTRY_CHARS_MAX, CAPABILITY_PATHS_MAX, CAPABILITY_TOOLS_MAX,
    CHILDREN_PER_TASK_MAX_DEFAULT, CapabilitySet, DEPENDENCY_IDS_MAX, DEPTH_MAX_DEFAULT,
    DIGEST_CHARS_MAX, ExperimentId, ID_CHARS_MAX, NodeId, NodeParams, NodeState,
    QUEUE_CAPACITY_DEFAULT, REVISION_CHARS_MAX, RunId, Scheduler, SchedulerLimits, SchedulerNode,
    TASK_DEADLINE_MS_DEFAULT, WORKERS_MAX_DEFAULT, WorkerRole, WriteAccess,
};
pub use error::ExperimentError;
pub use evaluator::{
    ARG_ENTRY_CHARS_MAX, ARTIFACTS_MAX, ArtifactDigest, BudgetTracker, CHECK_ARGV_MAX, CHECKS_MAX,
    COVERAGE_MAX, CheckRun, EVIDENCE_NAME_CHARS_MAX, EvalStage, Evaluator, EvidenceBundle,
    STAGE_TRANSITIONS_MAX, VerificationOutcome,
};
pub use manifest::{
    Acceptance, BudgetDefaults, BudgetManifest, FORBIDDEN_PATHS_MAX, LANGUAGES_PER_TIER_MAX,
    LanguageDef, LanguageManifest, MANIFEST_DESC_CHARS_MAX, MANIFEST_NAME_CHARS_MAX,
    MANIFEST_SCHEMA_VERSION, PromotionManifest, PromotionThresholds, REQUIRED_FILES_MAX, RiskClass,
    SUITES_MAX, SuiteDef, SuiteManifest, TASK_CHECKS_MAX, TIERS_MAX, TaskBudget, TaskCheck,
    TaskManifest, TierDef, parse_budget_manifest, parse_language_manifest,
    parse_promotion_manifest, parse_suite_manifest, parse_task_manifest, read_manifest_file,
};
pub use promotion::{
    APPROVAL_FIELD_CHARS_MAX, APPROVAL_SCOPE_CHARS_MAX, CANDIDATE_DIGEST_HEX_MIN,
    CHANGED_SURFACE_MAX, DIFF_SUMMARY_CHARS_MAX, DIGEST_HEX_CHARS_MAX, HumanApproval,
    ImprovementProposal, Lifecycle, LifecycleEvent, OPERATOR_RECORD_CHARS_MAX,
    PROPOSAL_TEXT_CHARS_MAX, PromotionGate, PromotionRecord, ProposalBudgets, ProposalParams,
    REVIEWER_DECISIONS_MAX, ReviewDecision, ReviewerDecision, SURFACE_PATH_CHARS_MAX,
    check_proposal_surface,
};
pub use record::{
    CHANGED_FILES_MAX, CheckRecord, EVENTS_MAX, EvaluationRecord, PromotionSection,
    RECORD_CHECKS_MAX, RECORD_LIST_MAX, RECORD_TEXT_CHARS_MAX, RecordParams, SCHEMA_VERSION,
    VerificationSection,
};
