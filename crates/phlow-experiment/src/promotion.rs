//! Promotion pipeline: lifecycle, human approval, proposals, the gate.
//!
//! Hard constraints, enforced by types:
//!
//! - [`Lifecycle::transition`] is the only way to move a candidate; invalid
//!   transitions are typed errors and terminal states reject everything.
//! - [`HumanApproval`] is opaque and constructible only from a
//!   shape-validated operator record. There is no constructor path from
//!   model output — adversarial tests try and fail.
//! - [`PromotionGate::promote`] consumes the approval by value (one-time
//!   use) and fails closed when evidence is incomplete, the changed surface
//!   touches protected paths, or any reviewer did not approve.
//!
//! Real cryptographic signature verification of the operator record is a
//! future promotion gate; the current shape check only rejects malformed or
//! implausible records.

use crate::error::ExperimentError;
use crate::evaluator::EvidenceBundle;
use crate::manifest::RiskClass;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Bounds (all with units)
// ---------------------------------------------------------------------------

/// Maximum characters in an operator approval record.
pub const OPERATOR_RECORD_CHARS_MAX: usize = 4_096;
/// Maximum characters in the operator name or approval id.
pub const APPROVAL_FIELD_CHARS_MAX: usize = 64;
/// Maximum characters in the approval scope string.
pub const APPROVAL_SCOPE_CHARS_MAX: usize = 128;
/// Minimum/maximum hex characters in the candidate digest.
pub const CANDIDATE_DIGEST_HEX_MIN: usize = 16;
/// Maximum characters in a hex digest field.
pub const DIGEST_HEX_CHARS_MAX: usize = 128;
/// Maximum characters in a proposal's candidate diff summary.
pub const DIFF_SUMMARY_CHARS_MAX: usize = 4_096;
/// Maximum changed-surface paths on one proposal.
pub const CHANGED_SURFACE_MAX: usize = 64;
/// Maximum characters in one changed-surface path.
pub const SURFACE_PATH_CHARS_MAX: usize = 512;
/// Maximum reviewer decisions on one proposal.
pub const REVIEWER_DECISIONS_MAX: usize = 16;
/// Maximum characters in a proposal text field.
pub const PROPOSAL_TEXT_CHARS_MAX: usize = 2_048;

/// Keys an operator approval record must carry — exactly these, no more.
const APPROVAL_KEYS: &[&str] = &[
    "operator",
    "approval_id",
    "candidate",
    "scope",
    "expires_ms",
    "signature",
];

/// Changed-surface prefixes a candidate must never control: hidden holdouts,
/// immutable safety cases, the evaluator, the promotion gate, and the
/// promotion thresholds. A proposal touching any of these is rejected by
/// [`check_proposal_surface`] before any other promotion logic runs.
const PROTECTED_PREFIXES: &[&str] = &[
    "evals/holdout",
    "evals/safety",
    "src/evaluator.rs",
    "src/promotion.rs",
    "manifests/promotion.toml",
];

// ---------------------------------------------------------------------------
// Candidate lifecycle
// ---------------------------------------------------------------------------

/// The candidate lifecycle from the plan's state diagram.
///
/// Text equivalent of the contract: no candidate reaches the accepted
/// baseline without isolated execution, deterministic verification,
/// independent review, and explicit human approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// Proposed; not yet validated.
    Proposed,
    /// Candidate workspace created in isolation.
    Isolated,
    /// Required checks completed.
    Tested,
    /// Independent evaluation completed.
    Reviewed,
    /// All promotion gates satisfied; waiting on a human.
    AwaitingHuman,
    /// Explicitly human-approved.
    Promoted,
    /// Deployed to canary and monitored.
    Monitored,
    /// Terminal: rejected at any gate before promotion.
    Rejected,
    /// Terminal: rolled back after a regression or incident.
    RolledBack,
}

impl Lifecycle {
    /// The stable machine-readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Isolated => "isolated",
            Self::Tested => "tested",
            Self::Reviewed => "reviewed",
            Self::AwaitingHuman => "awaiting_human",
            Self::Promoted => "promoted",
            Self::Monitored => "monitored",
            Self::Rejected => "rejected",
            Self::RolledBack => "rolled_back",
        }
    }

    /// True for the two terminal states: Rejected and RolledBack.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Rejected | Self::RolledBack)
    }

    /// Applies `event` to the lifecycle.
    ///
    /// Accepted: exactly the transitions in the plan's diagram. Rejected:
    /// any other pair ([`ExperimentError::BadTransition`]); any event on a
    /// terminal state ([`ExperimentError::LifecycleTerminal`]). Rejection
    /// before promotion leaves the accepted baseline unchanged.
    pub fn transition(self, event: LifecycleEvent) -> Result<Lifecycle, ExperimentError> {
        if self.is_terminal() {
            return Err(ExperimentError::LifecycleTerminal {
                state: self.name(),
            });
        }
        let next = match (self, event) {
            (Self::Proposed, LifecycleEvent::ContractInvalid) => Self::Rejected,
            (Self::Proposed, LifecycleEvent::WorkspaceCreated) => Self::Isolated,
            (Self::Isolated, LifecycleEvent::ChecksComplete) => Self::Tested,
            (Self::Isolated, LifecycleEvent::ChecksFailed) => Self::Rejected,
            (Self::Tested, LifecycleEvent::EvaluationComplete) => Self::Reviewed,
            (Self::Tested, LifecycleEvent::RegressionFound) => Self::Rejected,
            (Self::Reviewed, LifecycleEvent::GatesSatisfied) => Self::AwaitingHuman,
            (Self::Reviewed, LifecycleEvent::RegressionFound) => Self::Rejected,
            (Self::AwaitingHuman, LifecycleEvent::HumanApproved) => Self::Promoted,
            (Self::AwaitingHuman, LifecycleEvent::ApprovalDenied) => Self::Rejected,
            (Self::AwaitingHuman, LifecycleEvent::ApprovalExpired) => Self::Rejected,
            (Self::Promoted, LifecycleEvent::DeployedToCanary) => Self::Monitored,
            (Self::Monitored, LifecycleEvent::RegressionDetected) => Self::RolledBack,
            _ => {
                return Err(ExperimentError::BadTransition {
                    from: self.name(),
                    event: event.name(),
                });
            }
        };
        Ok(next)
    }
}

/// Events that move the candidate lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// The proposal's contract or budget was invalid.
    ContractInvalid,
    /// The isolated candidate workspace was created.
    WorkspaceCreated,
    /// Required checks completed.
    ChecksComplete,
    /// Checks timed out, crashed, or failed.
    ChecksFailed,
    /// Independent evaluation completed.
    EvaluationComplete,
    /// A regression or unsafe behavior was found.
    RegressionFound,
    /// All promotion gates satisfied.
    GatesSatisfied,
    /// A human explicitly approved.
    HumanApproved,
    /// A human denied approval.
    ApprovalDenied,
    /// The approval expired before use.
    ApprovalExpired,
    /// The candidate was deployed to canary.
    DeployedToCanary,
    /// A regression or incident was detected in monitoring.
    RegressionDetected,
}

impl LifecycleEvent {
    /// The stable machine-readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::ContractInvalid => "contract_invalid",
            Self::WorkspaceCreated => "workspace_created",
            Self::ChecksComplete => "checks_complete",
            Self::ChecksFailed => "checks_failed",
            Self::EvaluationComplete => "evaluation_complete",
            Self::RegressionFound => "regression_found",
            Self::GatesSatisfied => "gates_satisfied",
            Self::HumanApproved => "human_approved",
            Self::ApprovalDenied => "approval_denied",
            Self::ApprovalExpired => "approval_expired",
            Self::DeployedToCanary => "deployed_to_canary",
            Self::RegressionDetected => "regression_detected",
        }
    }
}

// ---------------------------------------------------------------------------
// Human approval: opaque, operator-only
// ---------------------------------------------------------------------------

/// An operator's approval: an opaque token constructible only from a
/// shape-validated operator record.
///
/// The fields are private and there is no `From<&str>`, no deserialization,
/// and no other constructor — model output cannot become an approval. The
/// only path is [`HumanApproval::from_operator_record`], which parses
/// `key: value` lines and validates shapes:
///
/// ```text
/// operator: jdoe
/// approval_id: APR-2026-0001
/// candidate: 9f2b3c4d5e6f708192a3b4c5d6e7f809
/// scope: phlow-experiment/promotion
/// expires_ms: 1893456000000
/// signature: <64 or 128 hex characters>
/// ```
///
/// Exactly these keys are accepted (unknown or duplicate keys are
/// rejected); `expires_ms` must be a positive integer; `signature` must be
/// 64 or 128 hex characters. This is a *shape* check only: real signature
/// verification against the operator's key, expiry enforcement against a
/// trusted clock, and replay protection are future promotion gates, and the
/// record must pass them before any real promotion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanApproval {
    operator: String,
    approval_id: String,
    candidate_digest: String,
    scope: String,
    expires_ms: u64,
    record_fingerprint: String,
}

impl HumanApproval {
    /// Parses and shape-validates an operator approval record.
    ///
    /// Accepted: the exact six-key format above with well-shaped values.
    /// Rejected: empty or oversized records, malformed lines, unknown or
    /// duplicate keys, missing keys, empty or over-long fields, non-numeric
    /// or zero `expires_ms`, non-hex or wrong-length `signature`.
    pub fn from_operator_record(record: &str) -> Result<Self, ExperimentError> {
        if record.is_empty() {
            return Err(ExperimentError::ApprovalRejected {
                reason: "empty operator record",
            });
        }
        if record.len() > OPERATOR_RECORD_CHARS_MAX {
            return Err(ExperimentError::TextTooLong {
                field: "operator record",
                max: OPERATOR_RECORD_CHARS_MAX,
                got: record.len(),
            });
        }
        let mut fields: BTreeMap<String, String> = BTreeMap::new();
        for line in record.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once(':').ok_or(ExperimentError::ApprovalRejected {
                reason: "malformed line",
            })?;
            let key = key.trim();
            let value = value.trim();
            if !APPROVAL_KEYS.contains(&key) {
                return Err(ExperimentError::ApprovalRejected {
                    reason: "unknown key",
                });
            }
            if fields.insert(key.to_string(), value.to_string()).is_some() {
                return Err(ExperimentError::ApprovalRejected {
                    reason: "duplicate key",
                });
            }
        }
        for key in APPROVAL_KEYS {
            if !fields.contains_key(*key) {
                return Err(ExperimentError::ApprovalRejected {
                    reason: "missing key",
                });
            }
        }
        let operator = Self::check_field(&fields, "operator")?;
        let approval_id = Self::check_field(&fields, "approval_id")?;
        let candidate_digest = Self::check_hex_field(
            &fields,
            "candidate",
            CANDIDATE_DIGEST_HEX_MIN,
            DIGEST_HEX_CHARS_MAX,
        )?;
        let scope = Self::check_scoped_field(&fields, "scope")?;
        let expires_ms: u64 =
            fields["expires_ms"]
                .parse()
                .map_err(|_| ExperimentError::ApprovalRejected {
                    reason: "expires_ms not an integer",
                })?;
        if expires_ms == 0 {
            return Err(ExperimentError::ApprovalRejected {
                reason: "expires_ms must be positive",
            });
        }
        let signature = Self::check_hex_field(&fields, "signature", 64, DIGEST_HEX_CHARS_MAX)?;
        if signature.len() != 64 && signature.len() != DIGEST_HEX_CHARS_MAX {
            return Err(ExperimentError::ApprovalRejected {
                reason: "signature must be 64 or 128 hex characters",
            });
        }
        // Non-cryptographic binding fingerprint of the canonical record, so
        // the token is bound to exactly the record it was parsed from. This
        // is not a signature check; see the struct docs.
        let record_fingerprint = fnv1a_hex(record.as_bytes());
        Ok(Self {
            operator,
            approval_id,
            candidate_digest,
            scope,
            expires_ms,
            record_fingerprint,
        })
    }

    /// The operator who approved.
    pub fn operator(&self) -> &str {
        &self.operator
    }

    /// The approval id.
    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }

    /// The candidate digest the approval covers.
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }

    /// The scope the approval covers.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The expiry instant in milliseconds (enforcement is a future gate).
    pub fn expires_ms(&self) -> u64 {
        self.expires_ms
    }

    fn check_field(
        fields: &BTreeMap<String, String>,
        key: &str,
    ) -> Result<String, ExperimentError> {
        let value = &fields[key];
        if value.is_empty() {
            return Err(ExperimentError::ApprovalRejected {
                reason: "empty field",
            });
        }
        if value.len() > APPROVAL_FIELD_CHARS_MAX {
            return Err(ExperimentError::TextTooLong {
                field: "approval field",
                max: APPROVAL_FIELD_CHARS_MAX,
                got: value.len(),
            });
        }
        if !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        {
            return Err(ExperimentError::ApprovalRejected {
                reason: "field has unexpected characters",
            });
        }
        Ok(value.clone())
    }

    fn check_scoped_field(
        fields: &BTreeMap<String, String>,
        key: &str,
    ) -> Result<String, ExperimentError> {
        let value = &fields[key];
        if value.is_empty() {
            return Err(ExperimentError::ApprovalRejected {
                reason: "empty field",
            });
        }
        if value.len() > APPROVAL_SCOPE_CHARS_MAX {
            return Err(ExperimentError::TextTooLong {
                field: "approval scope",
                max: APPROVAL_SCOPE_CHARS_MAX,
                got: value.len(),
            });
        }
        if !value.bytes().all(|b| {
            b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'/'
        }) {
            return Err(ExperimentError::ApprovalRejected {
                reason: "scope has unexpected characters",
            });
        }
        Ok(value.clone())
    }

    fn check_hex_field(
        fields: &BTreeMap<String, String>,
        key: &str,
        min: usize,
        max: usize,
    ) -> Result<String, ExperimentError> {
        let value = &fields[key];
        if value.len() < min || value.len() > max {
            return Err(ExperimentError::ApprovalRejected {
                reason: "hex field has wrong length",
            });
        }
        if !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ExperimentError::ApprovalRejected {
                reason: "hex field has non-hex characters",
            });
        }
        Ok(value.clone())
    }
}

/// FNV-1a 64-bit, rendered as 16 hex characters.
///
/// Used only as a non-cryptographic binding fingerprint tying a
/// [`HumanApproval`] to the exact record bytes it was parsed from. It is
/// not a hash for security purposes and does not replace signature
/// verification.
fn fnv1a_hex(bytes: &[u8]) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(16);
    for shift in (0..64).step_by(8).rev() {
        let b = ((hash >> shift) & 0xff) as u8;
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------------
// Improvement proposals
// ---------------------------------------------------------------------------

/// A reviewer's structured decision on a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    /// The reviewer approves the proposal as-is.
    Approve,
    /// The reviewer requests changes before promotion.
    RequestChanges,
    /// The reviewer rejects the proposal.
    Reject,
}

/// One reviewer's decision, paired with their role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewerDecision {
    /// Which role decided.
    pub reviewer: crate::control_plane::WorkerRole,
    /// What they decided.
    pub decision: ReviewDecision,
}

/// Budgets the proposal ran under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProposalBudgets {
    /// Maximum tool calls the proposal's evaluation could consume.
    pub tool_calls_max: u64,
    /// Maximum wall time for the evaluation, in milliseconds.
    pub wall_ms_max: u64,
}

/// Constructor parameters for [`ImprovementProposal`].
#[derive(Debug, Clone)]
pub struct ProposalParams {
    /// What triggered the proposal (e.g. a failure classification id).
    pub trigger: String,
    /// The failure category being addressed.
    pub failure_category: String,
    /// The pinned baseline revision the candidate was built against.
    pub baseline_revision: String,
    /// Bounded human-readable summary of the candidate diff.
    pub candidate_diff_summary: String,
    /// Repo-relative paths the candidate changed.
    pub changed_surface: Vec<String>,
    /// The expected benefit, stated measurably.
    pub expected_benefit: String,
    /// The proposal's risk class.
    pub risk_class: RiskClass,
    /// The budgets the evaluation ran under.
    pub budgets: ProposalBudgets,
    /// The test-suite version the candidate was evaluated with.
    pub test_version: String,
    /// The evaluator version the candidate was evaluated with.
    pub evaluator_version: String,
    /// Summary of the evaluation results.
    pub results_summary: String,
    /// The independent reviewers' structured decisions.
    pub reviewer_decisions: Vec<ReviewerDecision>,
    /// The exact revision to restore on rollback.
    pub rollback_target: String,
}

/// A proposal to improve Phlow itself, built only as data.
///
/// The proposal carries the trigger, failure category, baseline revision,
/// candidate diff summary, changed surface, expected benefit, risk class,
/// budgets, test/evaluator versions, results, reviewer decisions, and
/// rollback target — the plan's required fields. Constructing one performs
/// no mutation; promotion requires [`PromotionGate::promote`] with a
/// [`HumanApproval`] and complete evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementProposal {
    trigger: String,
    failure_category: String,
    baseline_revision: String,
    candidate_diff_summary: String,
    changed_surface: Vec<String>,
    expected_benefit: String,
    risk_class: RiskClass,
    budgets: ProposalBudgets,
    test_version: String,
    evaluator_version: String,
    results_summary: String,
    reviewer_decisions: Vec<ReviewerDecision>,
    rollback_target: String,
}

impl ImprovementProposal {
    /// Builds a proposal after validating every field.
    ///
    /// Accepted: non-empty text fields within bounds, a non-empty
    /// changed surface within bounds, positive budgets, at least one
    /// reviewer decision. Rejected: empty or over-long fields, an empty or
    /// oversized changed surface, zero budgets, no reviewer decisions.
    /// Surface *policy* (protected paths) is checked separately by
    /// [`check_proposal_surface`], which the promotion gate always runs.
    pub fn new(params: ProposalParams) -> Result<Self, ExperimentError> {
        let trigger = check_text("trigger", &params.trigger)?;
        let failure_category = check_text("failure_category", &params.failure_category)?;
        let baseline_revision = check_text("baseline_revision", &params.baseline_revision)?;
        if params.candidate_diff_summary.is_empty() {
            return Err(ExperimentError::EmptyField {
                field: "candidate_diff_summary",
            });
        }
        if params.candidate_diff_summary.len() > DIFF_SUMMARY_CHARS_MAX {
            return Err(ExperimentError::TextTooLong {
                field: "candidate_diff_summary",
                max: DIFF_SUMMARY_CHARS_MAX,
                got: params.candidate_diff_summary.len(),
            });
        }
        if params.changed_surface.is_empty() {
            return Err(ExperimentError::EmptyField {
                field: "changed_surface",
            });
        }
        if params.changed_surface.len() > CHANGED_SURFACE_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "changed_surface",
                max: CHANGED_SURFACE_MAX,
            });
        }
        for path in &params.changed_surface {
            if path.len() > SURFACE_PATH_CHARS_MAX {
                return Err(ExperimentError::TextTooLong {
                    field: "changed_surface path",
                    max: SURFACE_PATH_CHARS_MAX,
                    got: path.len(),
                });
            }
        }
        let expected_benefit = check_text("expected_benefit", &params.expected_benefit)?;
        if params.budgets.tool_calls_max == 0 {
            return Err(ExperimentError::InvalidBudget {
                field: "proposal tool_calls_max",
            });
        }
        if params.budgets.wall_ms_max == 0 {
            return Err(ExperimentError::InvalidBudget {
                field: "proposal wall_ms_max",
            });
        }
        let test_version = check_text("test_version", &params.test_version)?;
        let evaluator_version = check_text("evaluator_version", &params.evaluator_version)?;
        let results_summary = check_text("results_summary", &params.results_summary)?;
        if params.reviewer_decisions.is_empty() {
            return Err(ExperimentError::EmptyField {
                field: "reviewer_decisions",
            });
        }
        if params.reviewer_decisions.len() > REVIEWER_DECISIONS_MAX {
            return Err(ExperimentError::TooManyItems {
                field: "reviewer_decisions",
                max: REVIEWER_DECISIONS_MAX,
            });
        }
        let rollback_target = check_text("rollback_target", &params.rollback_target)?;
        Ok(Self {
            trigger,
            failure_category,
            baseline_revision,
            candidate_diff_summary: params.candidate_diff_summary,
            changed_surface: params.changed_surface,
            expected_benefit,
            risk_class: params.risk_class,
            budgets: params.budgets,
            test_version,
            evaluator_version,
            results_summary,
            reviewer_decisions: params.reviewer_decisions,
            rollback_target,
        })
    }

    /// The repo-relative paths the candidate changed.
    pub fn changed_surface(&self) -> &[String] {
        &self.changed_surface
    }

    /// The reviewer decisions.
    pub fn reviewer_decisions(&self) -> &[ReviewerDecision] {
        &self.reviewer_decisions
    }

    /// The risk class.
    pub fn risk_class(&self) -> RiskClass {
        self.risk_class
    }

    /// The baseline revision.
    pub fn baseline_revision(&self) -> &str {
        &self.baseline_revision
    }

    /// The rollback target revision.
    pub fn rollback_target(&self) -> &str {
        &self.rollback_target
    }
}

/// Validates one proposal text field: non-empty, within
/// [`PROPOSAL_TEXT_CHARS_MAX`] characters.
fn check_text(field: &'static str, value: &str) -> Result<String, ExperimentError> {
    if value.is_empty() {
        return Err(ExperimentError::EmptyField { field });
    }
    if value.len() > PROPOSAL_TEXT_CHARS_MAX {
        return Err(ExperimentError::TextTooLong {
            field,
            max: PROPOSAL_TEXT_CHARS_MAX,
            got: value.len(),
        });
    }
    Ok(value.to_string())
}

/// Rejects proposals touching surfaces a candidate must never control.
///
/// Denied: any changed-surface entry equal to or under `evals/holdout`,
/// `evals/safety`, `src/evaluator.rs`, `src/promotion.rs`, or
/// `manifests/promotion.toml` ([`ExperimentError::ProtectedSurface`]);
/// empty, absolute, or parent-escaping paths
/// ([`ExperimentError::BadPath`]/[`ExperimentError::EmptyField`]).
/// This runs inside [`PromotionGate::promote`] and is also public so
/// earlier pipeline stages can fail fast on the same policy.
pub fn check_proposal_surface(proposal: &ImprovementProposal) -> Result<(), ExperimentError> {
    for path in proposal.changed_surface() {
        if path.is_empty() {
            return Err(ExperimentError::EmptyField {
                field: "changed_surface entry",
            });
        }
        if path.starts_with('/') || path.contains("..") {
            return Err(ExperimentError::BadPath {
                path: path.clone(),
            });
        }
        for prefix in PROTECTED_PREFIXES {
            if path == *prefix || path.starts_with(&format!("{prefix}/")) {
                return Err(ExperimentError::ProtectedSurface {
                    path: path.clone(),
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Promotion gate
// ---------------------------------------------------------------------------

/// The record produced when a proposal passes the promotion gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionRecord {
    candidate_digest: String,
    baseline_revision: String,
    rollback_target: String,
    operator: String,
    approval_id: String,
    evidence_checks: usize,
}

impl PromotionRecord {
    /// The candidate digest that was promoted.
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }
    /// The operator who approved.
    pub fn operator(&self) -> &str {
        &self.operator
    }
    /// The approval id consumed.
    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }
    /// The rollback target revision.
    pub fn rollback_target(&self) -> &str {
        &self.rollback_target
    }
    /// How many evidence checks the bundle carried.
    pub fn evidence_checks(&self) -> usize {
        self.evidence_checks
    }
}

/// The promotion gate: the single choke point before a candidate may be
/// presented for human-driven merge.
///
/// `promote` fails closed. It rejects: incomplete evidence
/// ([`ExperimentError::IncompleteEvidence`]); proposals touching protected
/// surfaces ([`ExperimentError::ProtectedSurface`],
/// [`ExperimentError::BadPath`]); any reviewer decision other than
/// [`ReviewDecision::Approve`] ([`ExperimentError::ReviewerRejected`]).
/// The approval is consumed by value — one token, one promotion — and there
/// is deliberately no path that promotes without one: the type system, not
/// a flag, enforces it.
pub struct PromotionGate;

impl PromotionGate {
    /// Attempts promotion. See the struct docs for the fail-closed rules.
    pub fn promote(
        proposal: &ImprovementProposal,
        approval: HumanApproval,
        evidence: &EvidenceBundle,
    ) -> Result<PromotionRecord, ExperimentError> {
        if !evidence.is_complete() {
            return Err(ExperimentError::IncompleteEvidence {
                missing: "complete evidence bundle",
            });
        }
        check_proposal_surface(proposal)?;
        if proposal.reviewer_decisions().is_empty() {
            return Err(ExperimentError::IncompleteEvidence {
                missing: "reviewer decisions",
            });
        }
        for decision in proposal.reviewer_decisions() {
            if decision.decision != ReviewDecision::Approve {
                return Err(ExperimentError::ReviewerRejected {
                    reviewer: decision.reviewer.name(),
                });
            }
        }
        Ok(PromotionRecord {
            candidate_digest: approval.candidate_digest().to_string(),
            baseline_revision: proposal.baseline_revision().to_string(),
            rollback_target: proposal.rollback_target().to_string(),
            operator: approval.operator().to_string(),
            approval_id: approval.approval_id().to_string(),
            evidence_checks: evidence.checks().len(),
        })
    }
}
