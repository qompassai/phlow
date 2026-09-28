//! task-96: cross-agent approval laundering (rust).
//!
//! The design attacks the delegation boundary: a subagent's approval is
//! not a human's approval — approvals must carry an authenticated human
//! principal, bind principal + action + proposal hash + run id, and
//! delegation must never widen approval authority.
//!
//! Seam mapping (verified, not invented): phlow-experiment's
//! [`HumanApproval`] is a dual-signed operator record binding
//! operator, approval id, candidate digest, scope, and expiry — and
//! nothing else. The record format accepts exactly eight keys; a
//! `run_id` key is rejected at parse as "unknown key", so run binding
//! is unrepresentable. [`PromotionGate::promote`] verifies the dual
//! signatures, rejects replays within one [`ConsumedApprovals`] store,
//! and rejects the acting agent == approving operator
//! ([`ExperimentError::SelfApproval`]) — but the acting agent is a
//! caller-supplied string (no authenticated principal), the approval's
//! candidate digest is copied into the record but never compared to the
//! proposal (task-91's banked gap), and there is no run-id binding at
//! all.
//!
//! Four cases: two validation, two adversarial.
//!
//! - V1 `genuine_approval_promotes`: the human operator's genuine
//!   approval, presented by the parent supervisor, promotes the
//!   approved action. PASSES.
//! - V2 `self_approval_rejected`: the approving operator acting as its
//!   own agent is rejected with `SelfApproval` — the one principal
//!   check that exists. PASSES.
//! - A1 `launder_across_actions`: the subagent replays the parent's
//!   genuine approval (covering digest X) to promote its OWN proposal
//!   Y — the promotion SUCCEEDS. The approval binds no proposal
//!   content (same missing binding as task-91's banked finding,
//!   attacked here across the delegation boundary) and the acting
//!   agent is an unenforced string. FAILS — the honest finding.
//! - A2 `launder_across_runs`: the genuine approval consumed in run A
//!   is forwarded into run B with a fresh replay store — the promotion
//!   SUCCEEDS. Cross-run replay protection depends on out-of-band
//!   store sharing, not on the approval: the record cannot carry a run
//!   id. FAILS — the honest finding.
//!
//! The task-level verdict is `fail` at `"launder_across_actions"`.
//!
//! Product decisions banked for Matt: (1) approval↔proposal content
//! binding (already banked from task-91 — same mechanism, new attack
//! vector); (2) run-id binding on approvals, so cross-run forwarding
//! is rejected by the record itself rather than by store-sharing
//! discipline.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{
    ArtifactDigest, CheckRun, ConsumedApprovals, EvidenceBundle, ExperimentError, HumanApproval,
    ImprovementProposal, ManualClock, OperatorRegistry, PromotionGate, ProposalBudgets,
    ProposalParams, ReviewDecision, ReviewerDecision, RiskClass, VerificationOutcome, WorkerRole,
};
use std::fmt;
use std::path::{Path, PathBuf};

/// The `signature::Keypair` trait (re-exported by `ml-dsa`), needed for
/// `verifying_key()` on the ML-DSA signing key.
use ml_dsa::Keypair as _;

/// Task id.
pub const ID: &str = "task-96";
/// Human-readable name.
pub const NAME: &str = "cross-agent approval laundering";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "genuine_approval_promotes",
    "self_approval_rejected",
    "launder_across_actions",
    "launder_across_runs",
];

// ---------------------------------------------------------------------------
// Bounds and fixture constants
// ---------------------------------------------------------------------------

/// 32-hex-char candidate digest the genuine approval covers (action X).
const CANDIDATE_DIGEST_HEX: &str = "9f2b3c4d5e6f708192a3b4c5d6e7f809";
/// Expiry for driver fixtures (2030-01-01T00:00:00Z).
const EXPIRES_MS: u64 = 1_893_456_000_000;
/// The parent supervisor presenting the human's approval.
const PARENT_AGENT: &str = "parent-supervisor";
/// The subagent attempting the laundering.
const SUBAGENT: &str = "subagent-1";
/// The enrolled human operator the fixtures sign as.
const OPERATOR: &str = "gauntlet-test-operator";
/// Approval scope used by every fixture record.
const SCOPE: &str = "phlow-experiment/task-96";

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-96 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-96: cannot build fixture {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

fn fixture_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Fixture {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Case verdicts
// ---------------------------------------------------------------------------

/// The parsed verdict of one case.
#[derive(Debug, Clone)]
pub struct CaseReport {
    /// Which case ran.
    pub case: String,
    /// Whether the case's own assertions held.
    pub passed: bool,
    /// Measured numbers.
    pub metrics: serde_json::Value,
    /// Diagnostic lines from the case.
    pub evidence: Vec<String>,
    /// Failing assertion details, empty when `passed`.
    pub failures: Vec<String>,
}

impl CaseReport {
    fn pass(case: &'static str, metrics: serde_json::Value, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: true,
            metrics,
            evidence,
            failures: Vec::new(),
        }
    }

    fn fail(case: &'static str, failure: String, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: false,
            metrics: serde_json::json!({}),
            evidence,
            failures: vec![failure],
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures: real dual-signed v2 records, generated in-driver
// ---------------------------------------------------------------------------

/// A deterministic Ed25519 + ML-DSA-65 keypair. Fixed test seeds — never
/// real key material — so fixtures are reproducible without randomness.
struct DriverKeypair {
    ed_sk: ed25519_dalek::SigningKey,
    pq_sk: ml_dsa::SigningKey<ml_dsa::MlDsa65>,
    ed_pk: [u8; 32],
    pq_pk: [u8; 1952],
}

fn driver_keypair(ed_seed: [u8; 32], pq_seed: [u8; 32]) -> DriverKeypair {
    let ed_sk = ed25519_dalek::SigningKey::from_bytes(&ed_seed);
    let seed = ml_dsa::Seed::from(pq_seed);
    let pq_sk = ml_dsa::SigningKey::<ml_dsa::MlDsa65>::from_seed(&seed);
    let ed_pk = ed_sk.verifying_key().to_bytes();
    let mut pq_pk = [0u8; 1952];
    pq_pk.copy_from_slice(pq_sk.verifying_key().encode().as_slice());
    DriverKeypair {
        ed_sk,
        pq_sk,
        ed_pk,
        pq_pk,
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// The exact canonical bytes the gate signs: six fields in fixed order,
/// `key: value` lines joined by LF, no trailing newline.
fn canonical_bytes(operator: &str, approval_id: &str, candidate_digest: &str) -> Vec<u8> {
    format!(
        "v: 2\n\
         operator: {operator}\n\
         approval_id: {approval_id}\n\
         candidate: {candidate_digest}\n\
         scope: {SCOPE}\n\
         expires_ms: {EXPIRES_MS}"
    )
    .into_bytes()
}

/// Builds a v2 operator record dual-signed with `keypair`, covering
/// `candidate_digest`: Ed25519 over the canonical bytes, ML-DSA-65 over
/// `canonical || ed_sig`.
fn signed_record(
    operator: &str,
    approval_id: &str,
    candidate_digest: &str,
    keypair: &DriverKeypair,
) -> String {
    use ml_dsa::Signer as _;
    let canonical = canonical_bytes(operator, approval_id, candidate_digest);
    let ed_sig = keypair.ed_sk.sign(&canonical);
    let mut pq_message = canonical;
    pq_message.extend_from_slice(&ed_sig.to_bytes());
    let pq_sig = keypair.pq_sk.sign(&pq_message);
    format!(
        "v: 2\n\
         operator: {operator}\n\
         approval_id: {approval_id}\n\
         candidate: {candidate_digest}\n\
         scope: {SCOPE}\n\
         expires_ms: {EXPIRES_MS}\n\
         signature_ed25519: {ed_hex}\n\
         signature_mldsa65: {pq_hex}\n",
        ed_hex = hex(&ed_sig.to_bytes()),
        pq_hex = hex(pq_sig.encode().as_slice()),
    )
}

/// Enrollment fingerprint: SHA-256 over `ed_pk || pq_pk`.
fn fingerprint(keypair: &DriverKeypair) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(keypair.ed_pk);
    hasher.update(keypair.pq_pk);
    hasher.finalize().into()
}

/// Writes a TOML operator registry enrolling `entries` under `dir`,
/// permissions 0600, and loads it through the real loader.
fn test_registry(dir: &Path, entries: &[(&str, &DriverKeypair, bool)]) -> OperatorRegistry {
    let path = dir.join("operators.toml");
    let mut toml = String::new();
    for (name, keypair, revoked) in entries {
        toml.push_str(&format!(
            "[operators.\"{name}\"]\n\
             ed25519_pubkey = \"{ed_hex}\"\n\
             mldsa65_pubkey = \"{pq_hex}\"\n\
             fingerprint = \"{fp_hex}\"\n\
             enrolled_ms = 1750000000000\n\
             revoked = {revoked}\n",
            ed_hex = hex(&keypair.ed_pk),
            pq_hex = hex(&keypair.pq_pk),
            fp_hex = hex(&fingerprint(keypair)),
        ));
    }
    std::fs::write(&path, &toml).expect("task-96: cannot write registry fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("task-96: cannot chmod registry fixture");
    }
    OperatorRegistry::load(&path).expect("task-96: registry fixture failed to load")
}

/// A fresh scratch dir for one case's registry file.
fn scratch_dir(case: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gauntlet-task-96-{case}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("task-96: cannot create scratch dir");
    dir
}

/// A complete evidence bundle: one passing check with exact argv, one
/// artifact digest, one verified outcome with coverage.
fn complete_evidence() -> Result<EvidenceBundle, ExperimentError> {
    let checks = vec![CheckRun::new(
        "tests",
        vec![
            "cargo".to_string(),
            "test".to_string(),
            "--locked".to_string(),
        ],
        true,
    )?];
    let artifacts = vec![ArtifactDigest::new("candidate.diff", "9f2b3c4d")?];
    let verification = VerificationOutcome::new(true, vec!["src/lib.rs".to_string()])?;
    EvidenceBundle::new(checks, artifacts, verification)
}

/// A minimal valid proposal: one unprotected changed file, one approving
/// reviewer, positive budgets. `trigger` distinguishes proposals.
fn valid_proposal(trigger: &str, surface: &str) -> Result<ImprovementProposal, ExperimentError> {
    ImprovementProposal::new(ProposalParams {
        trigger: trigger.to_string(),
        failure_category: "approval-binding".to_string(),
        baseline_revision: "87c182d".to_string(),
        candidate_diff_summary: "Harden the approval seam.".to_string(),
        changed_surface: vec![surface.to_string()],
        expected_benefit: "Clearer approval audit trail.".to_string(),
        risk_class: RiskClass::Normal,
        budgets: ProposalBudgets {
            tool_calls_max: 64,
            wall_ms_max: 300_000,
        },
        test_version: "gauntlet-1".to_string(),
        evaluator_version: "evaluator-1".to_string(),
        results_summary: "All gates green.".to_string(),
        reviewer_decisions: vec![ReviewerDecision {
            reviewer: WorkerRole::SecurityReviewer,
            decision: ReviewDecision::Approve,
        }],
        rollback_target: "87c182d".to_string(),
    })
}

/// Shared promotion inputs for one case: registry, keypair, clock.
struct PromotionInputs {
    keypair: DriverKeypair,
    registry: OperatorRegistry,
    clock: ManualClock,
}

fn promotion_inputs(case: &str) -> PromotionInputs {
    let keypair = driver_keypair([0x96; 32], [0x69; 32]);
    let dir = scratch_dir(case);
    let registry = test_registry(&dir, &[(OPERATOR, &keypair, false)]);
    let clock = ManualClock::new(EXPIRES_MS - 1_000);
    PromotionInputs {
        keypair,
        registry,
        clock,
    }
}

fn parse_record(record: &str, what: &str) -> Result<HumanApproval, DriverError> {
    HumanApproval::from_operator_record(record)
        .map_err(|e| fixture_error(what, format!("record rejected: {e}")))
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: the human operator's genuine approval, presented by the parent
/// supervisor, promotes the approved action.
fn case_genuine_approval_promotes() -> Result<CaseReport, DriverError> {
    const CASE: &str = "genuine_approval_promotes";
    let mut evidence = Vec::new();
    let inputs = promotion_inputs("genuine");
    let record = signed_record(
        OPERATOR,
        "APR-T96-0001",
        CANDIDATE_DIGEST_HEX,
        &inputs.keypair,
    );
    let approval = parse_record(&record, "genuine approval")?;
    let proposal = valid_proposal("TASK96-001", "src/approval_docs.rs")
        .map_err(|e| fixture_error("genuine proposal", format!("invalid: {e}")))?;
    let bundle = complete_evidence()
        .map_err(|e| fixture_error("genuine evidence", format!("invalid: {e}")))?;
    let mut consumed = ConsumedApprovals::new();
    match PromotionGate::promote(
        &proposal,
        approval,
        &bundle,
        PARENT_AGENT,
        &inputs.clock,
        &inputs.registry,
        &mut consumed,
    ) {
        Ok(record_out) => {
            evidence.push(format!(
                "parent-presented genuine approval promoted: approval_id={} operator={}",
                record_out.approval_id(),
                record_out.operator(),
            ));
            Ok(CaseReport::pass(
                CASE,
                serde_json::json!({"promoted": true}),
                evidence,
            ))
        }
        Err(error) => Ok(CaseReport::fail(
            CASE,
            format!("genuine promotion rejected: {error}"),
            evidence,
        )),
    }
}

/// V2: the approving operator acting as its own agent is rejected with
/// `SelfApproval` — the one principal check the gate performs.
fn case_self_approval_rejected() -> Result<CaseReport, DriverError> {
    const CASE: &str = "self_approval_rejected";
    let mut evidence = Vec::new();
    let inputs = promotion_inputs("selfapproval");
    let record = signed_record(
        OPERATOR,
        "APR-T96-0002",
        CANDIDATE_DIGEST_HEX,
        &inputs.keypair,
    );
    let approval = parse_record(&record, "self approval")?;
    let proposal = valid_proposal("TASK96-002", "src/approval_docs.rs")
        .map_err(|e| fixture_error("self proposal", format!("invalid: {e}")))?;
    let bundle =
        complete_evidence().map_err(|e| fixture_error("self evidence", format!("invalid: {e}")))?;
    let mut consumed = ConsumedApprovals::new();
    // The operator presents its own approval as the acting agent.
    match PromotionGate::promote(
        &proposal,
        approval,
        &bundle,
        OPERATOR,
        &inputs.clock,
        &inputs.registry,
        &mut consumed,
    ) {
        Err(ExperimentError::SelfApproval { operator }) => {
            evidence.push(format!("self-approval rejected: SelfApproval({operator})"));
            evidence.push(
                "this is the gate's only principal check — and the acting \
                 agent is a caller-supplied string, so its strength is the \
                 caller's honesty"
                    .to_string(),
            );
            Ok(CaseReport::pass(
                CASE,
                serde_json::json!({"self_approval_rejected": true}),
                evidence,
            ))
        }
        Err(error) => Ok(CaseReport::fail(
            CASE,
            format!("rejected with unexpected error: {error}"),
            evidence,
        )),
        Ok(_) => Ok(CaseReport::fail(
            CASE,
            "operator acting as its own agent was NOT rejected".to_string(),
            evidence,
        )),
    }
}

/// A1: the subagent replays the parent's GENUINE approval (covering
/// digest X, dual signatures valid, operator enrolled) to promote its
/// OWN proposal Y — unrelated content, different trigger and surface.
/// The promotion succeeds: the approval binds no proposal content and
/// the acting agent is an unenforced string.
fn case_launder_across_actions() -> Result<CaseReport, DriverError> {
    const CASE: &str = "launder_across_actions";
    let mut evidence = Vec::new();
    let inputs = promotion_inputs("launder-actions");
    // The parent's genuine approval for action X (digest X).
    let record = signed_record(
        OPERATOR,
        "APR-T96-0003",
        CANDIDATE_DIGEST_HEX,
        &inputs.keypair,
    );
    // The subagent re-parses the parent's record and promotes its own
    // proposal Y: different trigger, different changed surface —
    // content unrelated to digest X.
    let approval = parse_record(&record, "laundered approval")?;
    let proposal_y = valid_proposal("TASK96-003-SUBAGENT", "src/subagent_feature.rs")
        .map_err(|e| fixture_error("subagent proposal", format!("invalid: {e}")))?;
    let bundle = complete_evidence()
        .map_err(|e| fixture_error("launder evidence", format!("invalid: {e}")))?;
    let mut consumed = ConsumedApprovals::new();
    match PromotionGate::promote(
        &proposal_y,
        approval,
        &bundle,
        SUBAGENT,
        &inputs.clock,
        &inputs.registry,
        &mut consumed,
    ) {
        Ok(record_out) => {
            evidence.push(format!(
                "LAUNDERED: subagent promoted its own proposal with the parent's approval \
                 (approval_id={}); the gate never compared the approval's candidate digest \
                 to the proposal",
                record_out.approval_id(),
            ));
            Ok(CaseReport::fail(
                CASE,
                "launder_across_actions: subagent promoted unrelated proposal Y with the \
                 parent's genuine approval for digest X — approvals bind no proposal content, \
                 and the acting agent is an unenforced caller-supplied string"
                    .to_string(),
                evidence,
            ))
        }
        Err(error) => {
            evidence.push(format!("laundering rejected: {error}"));
            Ok(CaseReport::pass(
                CASE,
                serde_json::json!({"laundering_rejected": true}),
                evidence,
            ))
        }
    }
}

/// A2: the genuine approval consumed in run A is forwarded into run B
/// (fresh replay store). The promotion succeeds: the record format
/// accepts exactly eight keys — a `run_id` key is rejected at parse as
/// "unknown key" — so run binding is unrepresentable, and cross-run
/// replay protection depends on out-of-band store sharing.
fn case_launder_across_runs() -> Result<CaseReport, DriverError> {
    const CASE: &str = "launder_across_runs";
    let mut evidence = Vec::new();
    let inputs = promotion_inputs("launder-runs");
    let record = signed_record(
        OPERATOR,
        "APR-T96-0004",
        CANDIDATE_DIGEST_HEX,
        &inputs.keypair,
    );
    // Structural premise: the record format cannot carry a run id.
    let with_run_id = format!("{record}run_id: run-b\n");
    match HumanApproval::from_operator_record(&with_run_id) {
        Err(ExperimentError::ApprovalRejected { reason }) => {
            evidence.push(format!(
                "record with a run_id key is rejected at parse: {reason} — \
                 run binding is unrepresentable in the v2 format"
            ));
        }
        Err(error) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("run_id probe rejected unexpectedly: {error}"),
                evidence,
            ));
        }
        Ok(_) => {
            return Ok(CaseReport::fail(
                CASE,
                "a run_id key parsed — the unrepresentable-run-binding premise changed".to_string(),
                evidence,
            ));
        }
    }
    // Run A: the parent consumes the approval normally.
    let approval_a = parse_record(&record, "run-A approval")?;
    let proposal = valid_proposal("TASK96-004", "src/approval_docs.rs")
        .map_err(|e| fixture_error("run proposal", format!("invalid: {e}")))?;
    let bundle =
        complete_evidence().map_err(|e| fixture_error("run evidence", format!("invalid: {e}")))?;
    let mut consumed_a = ConsumedApprovals::new();
    PromotionGate::promote(
        &proposal,
        approval_a,
        &bundle,
        PARENT_AGENT,
        &inputs.clock,
        &inputs.registry,
        &mut consumed_a,
    )
    .map_err(|e| fixture_error("run-A promotion", format!("unexpected rejection: {e}")))?;
    evidence.push("run A consumed APR-T96-0004".to_string());
    // Run B: the subagent forwards the same genuine record into a new
    // run with its own (fresh) replay store.
    let approval_b = parse_record(&record, "run-B approval")?;
    let mut consumed_b = ConsumedApprovals::new();
    match PromotionGate::promote(
        &proposal,
        approval_b,
        &bundle,
        SUBAGENT,
        &inputs.clock,
        &inputs.registry,
        &mut consumed_b,
    ) {
        Ok(record_out) => {
            evidence.push(format!(
                "LAUNDERED ACROSS RUNS: run B promoted with the run-A approval \
                 (approval_id={}) against a fresh replay store",
                record_out.approval_id(),
            ));
            Ok(CaseReport::fail(
                CASE,
                "launder_across_runs: a genuine approval consumed in run A promoted again in \
                 run B — the record carries no run binding, so cross-run replay protection \
                 depends on out-of-band replay-store sharing, not on the approval itself"
                    .to_string(),
                evidence,
            ))
        }
        Err(error) => {
            evidence.push(format!("cross-run replay rejected: {error}"));
            Ok(CaseReport::pass(
                CASE,
                serde_json::json!({"cross_run_replay_rejected": true}),
                evidence,
            ))
        }
    }
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "genuine_approval_promotes" => case_genuine_approval_promotes(),
        "self_approval_rejected" => case_self_approval_rejected(),
        "launder_across_actions" => case_launder_across_actions(),
        "launder_across_runs" => case_launder_across_runs(),
        _ => Err(fixture_error("case", format!("unknown case '{case}'"))),
    }
}

// ---------------------------------------------------------------------------
// Task entry point
// ---------------------------------------------------------------------------

struct TaskFailure {
    where_: String,
    how: String,
    evidence: Vec<String>,
}

fn run_inner() -> Result<Vec<String>, TaskFailure> {
    let mut evidence = vec![
        "seam: REAL but incomplete — phlow-experiment's HumanApproval binds \
         operator + approval id + candidate digest + scope + expiry; the \
         acting agent is a caller-supplied string; no run-id binding exists"
            .to_string(),
    ];
    for case in CASES {
        let report = run_case(case).map_err(|e| TaskFailure {
            where_: case.to_string(),
            how: e.to_string(),
            evidence: evidence.clone(),
        })?;
        evidence.push(format!("case {case}: passed={}", report.passed));
        evidence.push(format!("case {case} metrics: {}", report.metrics));
        for line in &report.evidence {
            evidence.push(format!("case {case}: {line}"));
        }
        if !report.passed {
            let mut how = report.failures.join("; ");
            how.push_str(
                " Product decisions banked for Matt: (1) approval↔proposal content binding \
                 (already banked from task-91 — same missing binding, attacked here across the \
                 delegation boundary); (2) run-id binding on approvals, so cross-run forwarding \
                 is rejected by the record itself rather than by replay-store-sharing discipline. \
                 Neither is implemented on gauntlet authority.",
            );
            return Err(TaskFailure {
                where_: case.to_string(),
                how,
                evidence,
            });
        }
    }
    Ok(evidence)
}

/// Attempt the task.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_inner() {
        Ok(evidence) => TaskOutcome::Pass {
            evidence: bound_evidence(evidence),
        },
        Err(failure) => TaskOutcome::Fail {
            where_: failure.where_,
            how: failure.how,
            evidence: bound_evidence(failure.evidence),
        },
    }
}
