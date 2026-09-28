//! task-91: propose→approve→apply pipeline (rust).
//!
//! The design asks for the sanctioned self-improvement path to work
//! end-to-end: a typed proposal (diff + rationale + tests) → human
//! approval bound to the proposal hash → apply → post-apply
//! verification. Apply requires a valid, hash-bound human approval with
//! no code path around it; every stage transition is recorded; replays
//! are idempotent-rejected.
//!
//! Seam mapping (verified, not invented): the pipeline is REAL —
//! `phlow-experiment`'s [`ImprovementProposal`] → [`HumanApproval`] →
//! [`PromotionGate::promote`] → [`PromotionRecord`]. The "apply" step is
//! deliberately human-driven merge (the hard constraint against
//! autonomous self-modification): the gate authorizes, it does not
//! mutate the tree, and the record carries the `rollback_target`
//! revision rather than an "applied tree hash".
//!
//! Four cases: two validation, two adversarial.
//!
//! - V1 `genuine_approval_promotes`: valid proposal + genuine
//!   dual-signed operator approval + complete evidence → promotion
//!   succeeds; the record links the candidate digest, approval id,
//!   operator, baseline revision, and rollback target. PASSES.
//! - V2 `no_approval_no_promotion_path`: `promote` takes the
//!   [`HumanApproval`] token by value — there is no `Option`, no
//!   default, no bypass; malformed records are rejected at parse.
//!   PASSES (by construction — stronger than a runtime check).
//! - A1 `approval_candidate_mismatch`: a GENUINE approval for candidate
//!   digest X promotes a proposal whose content is unrelated to X —
//!   the gate never compares the approval's candidate digest against
//!   the proposal, and `ImprovementProposal` carries no digest field
//!   to compare against. The design demands rejection with
//!   `approval_mismatch`; no such check (and no such error variant)
//!   exists. FAILS — the honest finding.
//! - A2 `replay_rejected`: the same approval id used twice → the second
//!   promotion fails with `ApprovalReplayed`. PASSES.
//!
//! The task-level verdict is `fail` at `"approval_candidate_mismatch"`:
//! the pipeline exists and three of four properties hold, but the
//! approval↔proposal hash binding the design demands is not verified by
//! the gate. Whether to add the check is Matt's call — banked, not
//! implemented on gauntlet authority.

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
pub const ID: &str = "task-91";
/// Human-readable name.
pub const NAME: &str = "propose→approve→apply pipeline";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "genuine_approval_promotes",
    "no_approval_no_promotion_path",
    "approval_candidate_mismatch",
    "replay_rejected",
];

// ---------------------------------------------------------------------------
// Bounds and fixture constants
// ---------------------------------------------------------------------------

/// 32-hex-char candidate digest the genuine approval covers.
const CANDIDATE_DIGEST_HEX: &str = "9f2b3c4d5e6f708192a3b4c5d6e7f809";
/// A different 32-hex-char digest: the approval in A1 covers THIS, while
/// the proposal's content is unrelated to it.
const OTHER_DIGEST_HEX: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
/// Expiry for driver fixtures (2030-01-01T00:00:00Z).
const EXPIRES_MS: u64 = 1_893_456_000_000;
/// The acting-agent identity for driver scenarios. Deliberately not the
/// enrolled operator (self-approval is task-11's territory).
const DRIVER_AGENT: &str = "gauntlet-runner";
/// The enrolled operator the fixtures sign as.
const OPERATOR: &str = "gauntlet-test-operator";
/// Approval scope used by every fixture record.
const SCOPE: &str = "phlow-experiment/task-91";

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-91 driver itself (not of the code under test).
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
                write!(f, "task-91: cannot build fixture {what}: {detail}")
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
    std::fs::write(&path, &toml).expect("task-91: cannot write registry fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("task-91: cannot chmod registry fixture");
    }
    OperatorRegistry::load(&path).expect("task-91: registry fixture failed to load")
}

/// A fresh scratch dir for one case's registry file.
fn scratch_dir(case: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gauntlet-task-91-{case}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("task-91: cannot create scratch dir");
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
/// reviewer, positive budgets.
fn valid_proposal() -> Result<ImprovementProposal, ExperimentError> {
    ImprovementProposal::new(ProposalParams {
        trigger: "TASK91-001".to_string(),
        failure_category: "approval-binding".to_string(),
        baseline_revision: "87c182d".to_string(),
        candidate_diff_summary: "Harden the approval seam.".to_string(),
        changed_surface: vec!["src/approval_docs.rs".to_string()],
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

/// A proposal whose content is unrelated to the candidate digest the A1
/// approval covers: different trigger, different diff summary, different
/// changed surface.
fn unrelated_proposal() -> Result<ImprovementProposal, ExperimentError> {
    ImprovementProposal::new(ProposalParams {
        trigger: "TASK91-002".to_string(),
        failure_category: "telemetry".to_string(),
        baseline_revision: "c0ffee1".to_string(),
        candidate_diff_summary: "Add a telemetry counter.".to_string(),
        changed_surface: vec!["src/telemetry.rs".to_string()],
        expected_benefit: "Better observability.".to_string(),
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
        rollback_target: "c0ffee1".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: valid proposal + genuine dual-signed operator approval + complete
/// evidence → promotion succeeds; the record links the candidate digest,
/// approval id, operator, baseline revision, and rollback target.
fn case_genuine_approval_promotes() -> Result<CaseReport, DriverError> {
    const CASE: &str = "genuine_approval_promotes";
    let mut evidence = Vec::new();
    let keypair = driver_keypair([0x42; 32], [0x24; 32]);
    let dir = scratch_dir("genuine");
    let registry = test_registry(&dir, &[(OPERATOR, &keypair, false)]);
    let record = signed_record(OPERATOR, "APR-T91-0001", CANDIDATE_DIGEST_HEX, &keypair);
    let approval = HumanApproval::from_operator_record(&record)
        .map_err(|e| fixture_error("genuine approval", format!("record rejected: {e}")))?;
    let proposal =
        valid_proposal().map_err(|e| fixture_error("genuine proposal", format!("invalid: {e}")))?;
    let bundle = complete_evidence()
        .map_err(|e| fixture_error("genuine evidence", format!("invalid: {e}")))?;
    let clock = ManualClock::new(EXPIRES_MS - 1_000);
    let mut consumed = ConsumedApprovals::new();
    let promotion = PromotionGate::promote(
        &proposal,
        approval,
        &bundle,
        DRIVER_AGENT,
        &clock,
        &registry,
        &mut consumed,
    );
    let record_out = match promotion {
        Ok(record_out) => record_out,
        Err(error) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("genuine promotion rejected: {error}"),
                evidence,
            ));
        }
    };
    evidence.push(format!(
        "promote OK: candidate_digest={}",
        record_out.candidate_digest()
    ));
    evidence.push(format!("approval_id={}", record_out.approval_id()));
    evidence.push(format!("operator={}", record_out.operator()));
    evidence.push(format!("rollback_target={}", record_out.rollback_target()));
    if record_out.candidate_digest() != CANDIDATE_DIGEST_HEX {
        return Ok(CaseReport::fail(
            CASE,
            "record candidate digest does not match the approval's".to_string(),
            evidence,
        ));
    }
    if record_out.approval_id() != "APR-T91-0001" || record_out.operator() != OPERATOR {
        return Ok(CaseReport::fail(
            CASE,
            "record does not link the approval id/operator".to_string(),
            evidence,
        ));
    }
    if record_out.rollback_target() != proposal.rollback_target() {
        return Ok(CaseReport::fail(
            CASE,
            "record does not link the proposal's rollback target".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the run record links proposal (baseline/rollback revisions), \
         approval (id, operator), and candidate digest; the apply step is \
         human-driven merge by design (no autonomous self-modification), \
         so no applied-tree hash exists in-band"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"evidence_checks": record_out.evidence_checks()}),
        evidence,
    ))
}

/// V2: there is no promotion path without a `HumanApproval` token. The
/// gate takes the approval by value — no `Option`, no default, no
/// bypass — and malformed records are rejected at parse. Stronger than
/// a runtime `approval_missing` check: the type system enforces it.
fn case_no_approval_no_promotion_path() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_approval_no_promotion_path";
    let mut evidence = Vec::new();
    match HumanApproval::from_operator_record("") {
        Err(ExperimentError::ApprovalRejected { reason }) => {
            evidence.push(format!("empty record rejected at parse: {reason}"));
        }
        Err(error) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("empty record rejected with unexpected error: {error}"),
                evidence,
            ));
        }
        Ok(_) => {
            return Ok(CaseReport::fail(
                CASE,
                "empty record parsed into an approval".to_string(),
                evidence,
            ));
        }
    }
    match HumanApproval::from_operator_record("not an operator record") {
        Err(_) => evidence.push("garbage record rejected at parse".to_string()),
        Ok(_) => {
            return Ok(CaseReport::fail(
                CASE,
                "garbage parsed into an approval".to_string(),
                evidence,
            ));
        }
    }
    evidence.push(
        "PromotionGate::promote takes `approval: HumanApproval` by value — \
         the only constructor is from_operator_record on a shaped record, \
         and the gate verifies the dual signatures; model output cannot \
         become an approval, and no code path promotes without one"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"bypass_paths": 0}),
        evidence,
    ))
}

/// A1: a GENUINE approval (valid dual signatures, enrolled operator)
/// covering candidate digest X is used to promote a proposal whose
/// content is unrelated to X. The design demands rejection with
/// `approval_mismatch`. The gate never compares the approval's
/// candidate digest against the proposal — `ImprovementProposal`
/// carries no digest field at all — so the promotion succeeds and the
/// record just copies the approval's digest. Honest FAIL.
fn case_approval_candidate_mismatch() -> Result<CaseReport, DriverError> {
    const CASE: &str = "approval_candidate_mismatch";
    let mut evidence = Vec::new();
    let keypair = driver_keypair([0x11; 32], [0x22; 32]);
    let dir = scratch_dir("mismatch");
    let registry = test_registry(&dir, &[(OPERATOR, &keypair, false)]);
    // Genuine approval for OTHER_DIGEST_HEX — valid signatures, enrolled
    // operator, live clock.
    let record = signed_record(OPERATOR, "APR-T91-0002", OTHER_DIGEST_HEX, &keypair);
    let approval = HumanApproval::from_operator_record(&record)
        .map_err(|e| fixture_error("mismatch approval", format!("record rejected: {e}")))?;
    evidence.push(format!(
        "approval covers candidate digest {OTHER_DIGEST_HEX} (genuine, dual-signed)"
    ));
    let proposal = unrelated_proposal()
        .map_err(|e| fixture_error("mismatch proposal", format!("invalid: {e}")))?;
    evidence.push(format!(
        "proposal content is unrelated: diff_summary={:?} baseline={}",
        "Add a telemetry counter.",
        proposal.baseline_revision()
    ));
    let bundle = complete_evidence()
        .map_err(|e| fixture_error("mismatch evidence", format!("invalid: {e}")))?;
    let clock = ManualClock::new(EXPIRES_MS - 1_000);
    let mut consumed = ConsumedApprovals::new();
    match PromotionGate::promote(
        &proposal,
        approval,
        &bundle,
        DRIVER_AGENT,
        &clock,
        &registry,
        &mut consumed,
    ) {
        Ok(record_out) => {
            evidence.push(format!(
                "promote SUCCEEDED with a mismatched approval; record.candidate_digest={} \
                 (copied from the approval, never compared to the proposal)",
                record_out.candidate_digest()
            ));
            Ok(CaseReport::fail(
                CASE,
                "gate promoted a proposal with an approval for a different candidate digest: \
                 no approval_mismatch check exists (ExperimentError has no such variant; \
                 ImprovementProposal carries no digest field to compare against)"
                    .to_string(),
                evidence,
            ))
        }
        Err(error) => {
            // Fail closed: if the gate ever rejects here, only a
            // mismatch-typed rejection satisfies the design.
            let message = error.to_string();
            evidence.push(format!("gate rejected: {message}"));
            if message.contains("mismatch") {
                Ok(CaseReport::pass(
                    CASE,
                    serde_json::json!({"rejected": message}),
                    evidence,
                ))
            } else {
                Ok(CaseReport::fail(
                    CASE,
                    format!("gate rejected, but not with approval_mismatch: {error}"),
                    evidence,
                ))
            }
        }
    }
}

/// A2: the same approval id used twice — the second promotion is
/// rejected with `ApprovalReplayed`. The by-value consumption plus the
/// replay store close both the token-reuse and the re-parse holes.
fn case_replay_rejected() -> Result<CaseReport, DriverError> {
    const CASE: &str = "replay_rejected";
    let mut evidence = Vec::new();
    let keypair = driver_keypair([0x33; 32], [0x44; 32]);
    let dir = scratch_dir("replay");
    let registry = test_registry(&dir, &[(OPERATOR, &keypair, false)]);
    let proposal =
        valid_proposal().map_err(|e| fixture_error("replay proposal", format!("invalid: {e}")))?;
    let bundle = complete_evidence()
        .map_err(|e| fixture_error("replay evidence", format!("invalid: {e}")))?;
    let clock = ManualClock::new(EXPIRES_MS - 1_000);
    let mut consumed = ConsumedApprovals::new();
    let record = signed_record(OPERATOR, "APR-T91-0003", CANDIDATE_DIGEST_HEX, &keypair);
    let approval = HumanApproval::from_operator_record(&record)
        .map_err(|e| fixture_error("replay approval", format!("record rejected: {e}")))?;
    PromotionGate::promote(
        &proposal,
        approval,
        &bundle,
        DRIVER_AGENT,
        &clock,
        &registry,
        &mut consumed,
    )
    .map_err(|e| fixture_error("first promotion", format!("unexpected rejection: {e}")))?;
    evidence.push("first promotion consumed APR-T91-0003".to_string());
    // Re-parse the same record text: a second token for the same id.
    let approval2 = HumanApproval::from_operator_record(&record)
        .map_err(|e| fixture_error("replay re-parse", format!("record rejected: {e}")))?;
    match PromotionGate::promote(
        &proposal,
        approval2,
        &bundle,
        DRIVER_AGENT,
        &clock,
        &registry,
        &mut consumed,
    ) {
        Err(ExperimentError::ApprovalReplayed { approval_id }) => {
            evidence.push(format!(
                "second promotion rejected: ApprovalReplayed({approval_id})"
            ));
            Ok(CaseReport::pass(
                CASE,
                serde_json::json!({"replay_rejected": true}),
                evidence,
            ))
        }
        Err(error) => Ok(CaseReport::fail(
            CASE,
            format!("second promotion rejected with unexpected error: {error}"),
            evidence,
        )),
        Ok(_) => Ok(CaseReport::fail(
            CASE,
            "second promotion with the same approval id succeeded".to_string(),
            evidence,
        )),
    }
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "genuine_approval_promotes" => case_genuine_approval_promotes(),
        "no_approval_no_promotion_path" => case_no_approval_no_promotion_path(),
        "approval_candidate_mismatch" => case_approval_candidate_mismatch(),
        "replay_rejected" => case_replay_rejected(),
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
        "seam: REAL — phlow-experiment's ImprovementProposal → HumanApproval \
         → PromotionGate::promote → PromotionRecord; the apply step is \
         human-driven merge by design (no autonomous self-modification)"
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
            if case == "approval_candidate_mismatch" {
                how.push_str(
                    " Product decision banked for Matt: whether to add proposal-content hashing \
                     and approval↔proposal digest verification is not implemented on gauntlet \
                     authority.",
                );
            }
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
