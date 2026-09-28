//! Promotion pipeline: lifecycle, human approval, proposals, the gate.
//!
//! Hard constraints, enforced by types:
//!
//! - [`Lifecycle::transition`] is the only way to move a candidate; invalid
//!   transitions are typed errors and terminal states reject everything.
//! - [`HumanApproval`] is opaque and constructible only from a
//!   dual-signature-verified operator record. There is no constructor path
//!   from model output — adversarial tests try and fail.
//! - [`PromotionGate::promote`] consumes the approval by value (one-time
//!   use) and fails closed when the acting agent is the approver, the
//!   approval id was already consumed, the operator is unknown or revoked,
//!   the record is expired or beyond the TTL policy, either signature fails,
//!   evidence is incomplete, the changed surface touches protected paths,
//!   or any reviewer did not approve.
//!
//! The operator record is format v2: eight keys, dual-signed with Ed25519
//! (classical) and ML-DSA-65 (post-quantum, FIPS 204) in a nested binding —
//! the PQ signature covers `canonical || ed25519_signature`, so the two
//! halves cannot be mixed across records. Verification is AND: both halves
//! must verify against the operator's registry-pinned keys. The old v1
//! shape-only records do not parse and are never verified.

use crate::error::ExperimentError;
use crate::evaluator::EvidenceBundle;
use crate::manifest::RiskClass;
use crate::registry::{OperatorKey, OperatorRegistry};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Bounds (all with units)
// ---------------------------------------------------------------------------

/// Maximum characters in an operator approval record. Sized for the v2
/// dual signature (6,746 hex chars) with headroom for field growth; still
/// bounded.
pub const OPERATOR_RECORD_CHARS_MAX: usize = 16_384;
/// Maximum characters in the operator name or approval id.
pub const APPROVAL_FIELD_CHARS_MAX: usize = 64;
/// Maximum characters in the approval scope string.
pub const APPROVAL_SCOPE_CHARS_MAX: usize = 128;
/// Minimum/maximum hex characters in the candidate digest.
pub const CANDIDATE_DIGEST_HEX_MIN: usize = 16;
/// Maximum characters in a hex digest field.
pub const DIGEST_HEX_CHARS_MAX: usize = 128;
/// Hex characters in an Ed25519 signature (64 bytes).
pub const ED25519_SIG_HEX_CHARS: usize = 128;
/// Hex characters in an ML-DSA-65 signature (3,309 bytes).
pub const MLDSA65_SIG_HEX_CHARS: usize = 6_618;
/// Maximum approval lifetime in milliseconds: 30 days. An expiry past
/// `now + APPROVAL_TTL_MAX_MS` is rejected outright — it bounds the blast
/// radius of a stolen approval. This is policy, not cryptography: a named
/// constant, easy to change when the operator sets operational policy.
pub const APPROVAL_TTL_MAX_MS: u64 = 2_592_000_000;
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

/// Keys a v2 operator approval record must carry — exactly these, no
/// more, in any order in the file (the canonical bytes use a fixed order).
const APPROVAL_KEYS: &[&str] = &[
    "v",
    "operator",
    "approval_id",
    "candidate",
    "scope",
    "expires_ms",
    "signature_ed25519",
    "signature_mldsa65",
];

/// The only record version this gate accepts. v1 shape-only records are
/// unparseable here by construction: they lack `v`, carry the unknown
/// `signature` key, and have no dual signatures to verify.
const APPROVAL_RECORD_VERSION: &str = "2";

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
            return Err(ExperimentError::LifecycleTerminal { state: self.name() });
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
/// dual-signature-verified operator record.
///
/// The fields are private and there is no `From<&str>`, no deserialization,
/// and no other constructor — model output cannot become an approval. The
/// only path is [`HumanApproval::from_operator_record`], which parses
/// `key: value` lines and validates shapes:
///
/// ```text
/// v: 2
/// operator: gauntlet-test-operator
/// approval_id: APR-2026-0001
/// candidate: 9f2b3c4d5e6f708192a3b4c5d6e7f809
/// scope: phlow-experiment/promotion
/// expires_ms: 1893456000000
/// signature_ed25519: <128 hex characters>
/// signature_mldsa65: <6618 hex characters>
/// ```
///
/// Exactly these keys are accepted (unknown or duplicate keys are
/// rejected); `v` must be `2`; `expires_ms` must be a positive integer;
/// the two signatures must be exact-length hex. The signatures themselves
/// are verified later, inside [`PromotionGate::promote`], against the
/// operator's registry-pinned keys: Ed25519 over the canonical bytes, then
/// ML-DSA-65 over `canonical || ed25519_signature` (nested binding), both
/// required. Parse accepts well-shaped records; only the gate decides
/// whether the seals are genuine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanApproval {
    operator: String,
    approval_id: String,
    candidate_digest: String,
    scope: String,
    expires_ms: u64,
    ed_sig: [u8; 64],
    pq_sig: [u8; 3309],
}

impl HumanApproval {
    /// Parses and shape-validates a v2 operator approval record.
    ///
    /// Accepted: the exact eight-key format above with well-shaped values.
    /// Rejected: empty or oversized records, malformed lines, unknown or
    /// duplicate keys, missing keys, a version other than `2`, empty or
    /// over-long fields, non-numeric or zero `expires_ms`, non-hex or
    /// wrong-length signatures. Shape validation is not authentication:
    /// [`PromotionGate::promote`] verifies the signatures.
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
            let (key, value) = line
                .split_once(':')
                .ok_or(ExperimentError::ApprovalRejected {
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
        if fields["v"] != APPROVAL_RECORD_VERSION {
            return Err(ExperimentError::ApprovalRejected {
                reason: "unsupported record version",
            });
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
        let ed_sig: [u8; 64] =
            Self::check_sig_field(&fields, "signature_ed25519", ED25519_SIG_HEX_CHARS)?;
        let pq_sig: [u8; 3309] =
            Self::check_sig_field(&fields, "signature_mldsa65", MLDSA65_SIG_HEX_CHARS)?;
        Ok(Self {
            operator,
            approval_id,
            candidate_digest,
            scope,
            expires_ms,
            ed_sig,
            pq_sig,
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

    /// The expiry instant in milliseconds (enforced by the gate against
    /// the trusted clock).
    pub fn expires_ms(&self) -> u64 {
        self.expires_ms
    }

    /// The exact bytes both signatures cover: the six non-signature
    /// fields in fixed order, `key: value` lines joined by LF, no trailing
    /// newline. Rebuilt from the parsed values, never sliced from the raw
    /// input — comments and blank lines cannot affect verification.
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut canonical = String::with_capacity(320);
        canonical.push_str("v: 2\n");
        canonical.push_str("operator: ");
        canonical.push_str(&self.operator);
        canonical.push_str("\napproval_id: ");
        canonical.push_str(&self.approval_id);
        canonical.push_str("\ncandidate: ");
        canonical.push_str(&self.candidate_digest);
        canonical.push_str("\nscope: ");
        canonical.push_str(&self.scope);
        canonical.push_str("\nexpires_ms: ");
        canonical.push_str(&self.expires_ms.to_string());
        canonical.into_bytes()
    }

    /// Verifies the dual signature against the operator's pinned keys.
    ///
    /// Ed25519 first (cheap, battle-tested) with `verify_strict`, which
    /// rejects malleable non-canonical signatures; then ML-DSA-65 over the
    /// nested binding `canonical || ed_sig`, so the two halves cannot be
    /// mixed across records. Both must verify: an attacker must break both
    /// schemes to forge. Each failure names its component.
    fn verify_dual_signatures(&self, key: &OperatorKey) -> Result<(), ExperimentError> {
        let canonical = self.canonical_bytes();
        let ed_signature = ed25519_dalek::Signature::from_bytes(&self.ed_sig);
        let ed_key = ed25519_dalek::VerifyingKey::from_bytes(&key.ed25519_pk).map_err(|_| {
            ExperimentError::BadSignature {
                component: "ed25519",
            }
        })?;
        ed_key
            .verify_strict(&canonical, &ed_signature)
            .map_err(|_| ExperimentError::BadSignature {
                component: "ed25519",
            })?;
        let mut pq_message = canonical;
        pq_message.extend_from_slice(&self.ed_sig);
        let pq_signature = ml_dsa::Signature::<ml_dsa::MlDsa65>::try_from(self.pq_sig.as_slice())
            .map_err(|_| ExperimentError::BadSignature {
            component: "mldsa65",
        })?;
        let encoded_key =
            ml_dsa::EncodedVerifyingKey::<ml_dsa::MlDsa65>::try_from(key.mldsa65_pk.as_slice())
                .map_err(|_| ExperimentError::BadSignature {
                    component: "mldsa65",
                })?;
        let pq_key = ml_dsa::VerifyingKey::<ml_dsa::MlDsa65>::decode(&encoded_key);
        use ml_dsa::Verifier as _;
        pq_key
            .verify(&pq_message, &pq_signature)
            .map_err(|_| ExperimentError::BadSignature {
                component: "mldsa65",
            })?;
        Ok(())
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
        if !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'/')
        {
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

    /// Validates a signature field: exactly `hex_chars` hex characters,
    /// decoded into `N` bytes. Wrong length or non-hex input is rejected
    /// before any cryptography runs.
    fn check_sig_field<const N: usize>(
        fields: &BTreeMap<String, String>,
        key: &str,
        hex_chars: usize,
    ) -> Result<[u8; N], ExperimentError> {
        let value = &fields[key];
        if value.len() != hex_chars || N * 2 != hex_chars {
            return Err(ExperimentError::ApprovalRejected {
                reason: "signature has wrong length",
            });
        }
        let mut out = [0u8; N];
        let (chunks, _) = value.as_bytes().as_chunks::<2>();
        for (index, pair) in chunks.iter().enumerate() {
            let hi = hex_nibble(pair[0]);
            let lo = hex_nibble(pair[1]);
            match (hi, lo) {
                (Some(hi), Some(lo)) => out[index] = (hi << 4) | lo,
                _ => {
                    return Err(ExperimentError::ApprovalRejected {
                        reason: "signature has non-hex characters",
                    });
                }
            }
        }
        Ok(out)
    }
}

/// One hex digit's value, or `None` for non-hex input.
fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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
            return Err(ExperimentError::BadPath { path: path.clone() });
        }
        for prefix in PROTECTED_PREFIXES {
            if path == *prefix || path.starts_with(&format!("{prefix}/")) {
                return Err(ExperimentError::ProtectedSurface { path: path.clone() });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Trusted clock
// ---------------------------------------------------------------------------

/// A trusted time source for approval expiry, in milliseconds since the
/// Unix epoch.
///
/// "Trusted" means the process's own clock, supplied by the gate's caller —
/// never parsed from a record, never influenced by the agent. The trait
/// (not a bare `SystemTime`) exists so expiry tests are deterministic via
/// [`ManualClock`].
pub trait Clock {
    /// Current time in milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
}

/// The production clock: the process wall clock.
///
/// Fail-closed: if the system clock is unavailable, this saturates to 0,
/// which makes every approval read as expired.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => duration.as_millis().try_into().unwrap_or(u64::MAX),
            Err(_) => 0,
        }
    }
}

/// A deterministic clock for tests: the time is whatever was set.
#[derive(Debug, Clone, Copy)]
pub struct ManualClock {
    now_ms: u64,
}

impl ManualClock {
    /// Builds a clock reading `now_ms`.
    pub fn new(now_ms: u64) -> Self {
        Self { now_ms }
    }

    /// Moves the clock to `now_ms`.
    pub fn set(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now_ms
    }
}

// ---------------------------------------------------------------------------
// Replay protection
// ---------------------------------------------------------------------------

/// Maximum bytes read from a consumed-approvals store file.
const CONSUMED_STORE_BYTES_MAX: u64 = 1_048_576;
/// Maximum characters in a stored approval id (matches the record bound).
const CONSUMED_ID_CHARS_MAX: usize = 64;

/// Single-use approval ids: the replay store.
///
/// `promote` already consumes the [`HumanApproval`] token by value (one
/// token, one call); this store closes the *re-parse* hole, where the
/// same record text parsed twice yields two tokens. Check-then-insert is
/// atomic within one `promote` call (single `&mut` borrow — no TOCTOU).
///
/// Entries live at most [`APPROVAL_TTL_MAX_MS`], so the store stays
/// bounded; [`ConsumedApprovals::evict_expired`] drops dead entries. For
/// restart safety the store persists as append-only JSONL, one
/// `{"id","expires_ms"}` object per line.
#[derive(Debug, Clone, Default)]
pub struct ConsumedApprovals {
    ids: std::collections::HashMap<String, u64>,
}

impl ConsumedApprovals {
    /// An empty replay store.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when `id` was already consumed.
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains_key(id)
    }

    /// Records `id` as consumed with its expiry. Call only after the
    /// approval passed every gate check.
    pub fn insert(&mut self, id: String, expires_ms: u64) {
        self.ids.insert(id, expires_ms);
    }

    /// Drops entries with `expires_ms <= now_ms`.
    pub fn evict_expired(&mut self, now_ms: u64) {
        self.ids.retain(|_, expires_ms| *expires_ms > now_ms);
    }

    /// How many ids are currently stored.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when no ids are stored.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The default store path: `PHLOW_CONSUMED_APPROVALS_FILE` wins, then
    /// `$XDG_DATA_HOME/phlow/consumed-approvals.jsonl`, then
    /// `~/.local/share/phlow/consumed-approvals.jsonl`.
    pub fn default_path() -> PathBuf {
        if let Ok(path) = std::env::var("PHLOW_CONSUMED_APPROVALS_FILE")
            && !path.is_empty()
        {
            return PathBuf::from(path);
        }
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME")
            && !xdg.is_empty()
        {
            return PathBuf::from(xdg).join("phlow/consumed-approvals.jsonl");
        }
        if let Ok(home) = std::env::var("HOME")
            && !home.is_empty()
        {
            return PathBuf::from(home).join(".local/share/phlow/consumed-approvals.jsonl");
        }
        PathBuf::from("phlow/consumed-approvals.jsonl")
    }

    /// Loads the store from a JSONL file, dropping expired entries as of
    /// `now_ms`. Malformed lines fail the load: the store is
    /// trust-adjacent, and silent skips would hide tampering.
    pub fn load(path: &Path, now_ms: u64) -> Result<Self, ExperimentError> {
        let metadata = std::fs::metadata(path).map_err(|_| ExperimentError::ApprovalRejected {
            reason: "consumed-approvals store unreadable",
        })?;
        if metadata.len() > CONSUMED_STORE_BYTES_MAX {
            return Err(ExperimentError::ApprovalRejected {
                reason: "consumed-approvals store too large",
            });
        }
        let text =
            std::fs::read_to_string(path).map_err(|_| ExperimentError::ApprovalRejected {
                reason: "consumed-approvals store unreadable",
            })?;
        let mut store = Self::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_str(line).map_err(|_| ExperimentError::ApprovalRejected {
                    reason: "consumed-approvals store is corrupt",
                })?;
            let id = value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty() && id.len() <= CONSUMED_ID_CHARS_MAX)
                .ok_or(ExperimentError::ApprovalRejected {
                    reason: "consumed-approvals store is corrupt",
                })?;
            let expires_ms = value
                .get("expires_ms")
                .and_then(serde_json::Value::as_u64)
                .ok_or(ExperimentError::ApprovalRejected {
                    reason: "consumed-approvals store is corrupt",
                })?;
            if store.ids.insert(id.to_string(), expires_ms).is_some() {
                return Err(ExperimentError::ApprovalRejected {
                    reason: "consumed-approvals store has a duplicate id",
                });
            }
        }
        store.evict_expired(now_ms);
        Ok(store)
    }

    /// Persists the store as JSONL, atomically (temp file + rename), one
    /// `{"id","expires_ms"}` object per line. Creates parent directories.
    pub fn save(&self, path: &Path) -> Result<(), ExperimentError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|_| ExperimentError::ApprovalRejected {
                reason: "consumed-approvals store directory unwritable",
            })?;
        }
        let mut text = String::new();
        let mut ids: Vec<(&String, &u64)> = self.ids.iter().collect();
        ids.sort_by(|a, b| a.0.cmp(b.0));
        for (id, expires_ms) in ids {
            text.push_str(&format!(
                "{{\"id\":{id_json},\"expires_ms\":{expires_ms}}}\n",
                id_json = serde_json::Value::String(id.clone()),
            ));
        }
        let tmp = path.with_extension("jsonl.tmp");
        std::fs::write(&tmp, &text).map_err(|_| ExperimentError::ApprovalRejected {
            reason: "consumed-approvals store unwritable",
        })?;
        std::fs::rename(&tmp, path).map_err(|_| ExperimentError::ApprovalRejected {
            reason: "consumed-approvals store unwritable",
        })?;
        Ok(())
    }
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
/// `promote` fails closed, in this exact check order (the order determines
/// which error surfaces first):
///
/// 1. Empty `acting_agent`, or `acting_agent == approval.operator()`:
///    [`ExperimentError::ApprovalRejected`] / [`ExperimentError::SelfApproval`].
///    The agent's identity is an explicit parameter — never an ambient or
///    global lookup — so the self-approval check cannot be dodged.
/// 2. Already-consumed `approval_id`: [`ExperimentError::ApprovalReplayed`].
/// 3. Registry lookup: [`ExperimentError::UnknownOperator`] /
///    [`ExperimentError::RevokedOperator`].
/// 4. Expiry against the trusted clock ([`ExperimentError::ApprovalExpired`])
///    and the TTL policy cap ([`ExperimentError::ExpiryBeyondMaxTtl`]).
/// 5. Dual signature verification ([`ExperimentError::BadSignature`]).
/// 6. The approval id is recorded as consumed — only after every check
///    passed.
/// 7. The pre-existing gates, unchanged: incomplete evidence
///    ([`ExperimentError::IncompleteEvidence`]); proposals touching
///    protected surfaces ([`ExperimentError::ProtectedSurface`],
///    [`ExperimentError::BadPath`]); any reviewer decision other than
///    [`ReviewDecision::Approve`] ([`ExperimentError::ReviewerRejected`]).
///
/// The approval is consumed by value — one token, one promotion — and the
/// replay store closes the re-parse hole. There is deliberately no path
/// that promotes without one: the type system, not a flag, enforces it.
pub struct PromotionGate;

impl PromotionGate {
    /// Attempts promotion. See the struct docs for the fail-closed check
    /// order.
    pub fn promote(
        proposal: &ImprovementProposal,
        approval: HumanApproval,
        evidence: &EvidenceBundle,
        acting_agent: &str,
        clock: &dyn Clock,
        registry: &OperatorRegistry,
        consumed: &mut ConsumedApprovals,
    ) -> Result<PromotionRecord, ExperimentError> {
        if acting_agent.is_empty() {
            return Err(ExperimentError::ApprovalRejected {
                reason: "empty acting agent",
            });
        }
        if approval.operator() == acting_agent {
            return Err(ExperimentError::SelfApproval {
                operator: approval.operator().to_string(),
            });
        }
        if consumed.contains(approval.approval_id()) {
            return Err(ExperimentError::ApprovalReplayed {
                approval_id: approval.approval_id().to_string(),
            });
        }
        let key = registry.lookup(approval.operator())?;
        let now_ms = clock.now_ms();
        let ttl_limit = now_ms.checked_add(APPROVAL_TTL_MAX_MS);
        if ttl_limit.is_none_or(|limit| approval.expires_ms() > limit) {
            return Err(ExperimentError::ExpiryBeyondMaxTtl);
        }
        if now_ms >= approval.expires_ms() {
            return Err(ExperimentError::ApprovalExpired);
        }
        approval.verify_dual_signatures(key)?;
        consumed.insert(approval.approval_id().to_string(), approval.expires_ms());
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
