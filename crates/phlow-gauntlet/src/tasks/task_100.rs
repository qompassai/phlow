//! task-100: artifact swap between pipeline stages (rust).
//!
//! The design demands the artifact be hash-chained across *every*
//! stage transition — propose → validate → approve → apply — so that
//! swapping it between any two stages is detected and fails closed
//! with `stage_artifact_mismatch`. The classic validated-twin swap.
//!
//! Seam mapping (verified, not invented): no stage chaining exists.
//! [`PromotionGate::promote`] takes the proposal, the [`HumanApproval`],
//! and the [`EvidenceBundle`] as three independent arguments and never
//! binds them to each other: the evidence bundle's artifact digests are
//! checked for completeness only, never compared to the proposal; the
//! approval's candidate digest is copied into the [`PromotionRecord`]
//! but never compared to the proposal (task-91's banked gap); and the
//! record itself carries no content hash of the approved artifact —
//! only the approval's digest, revision strings, operator, approval
//! id, and a check count. There is no apply stage in code at all:
//! apply is human-driven merge by design (task-91), and no `fn apply`
//! exists anywhere in `crates/phlow-experiment/src` — so no stage
//! re-hashes the artifact after approval.
//!
//! Four cases: two validation, two adversarial.
//!
//! - V1 `same_artifact_all_stages`: the benign artifact through
//!   propose → validate → approve succeeds — the unchained pipeline
//!   works when nothing is swapped. PASSES.
//! - V2 `record_carries_no_content_hash`: the emitted
//!   [`PromotionRecord`] carries the approval's candidate digest
//!   (copied, uncompared) and no hash computed over the proposal's
//!   content — the structural reason swaps are undetectable. PASSES.
//! - A1 `swap_between_validate_and_approve`: the evidence bundle is
//!   built for the BENIGN twin while the proposal promoted is the
//!   MALICIOUS twin — the promotion SUCCEEDS. The validate→approve
//!   binding is absent. FAILS — the honest finding.
//! - A2 `swap_between_approve_and_apply`: after a successful
//!   promotion, nothing in-band re-verifies the artifact before the
//!   human-driven apply — no apply-stage API exists to reject a
//!   swapped artifact, filename-preserving or otherwise. FAILS — the
//!   honest finding.
//!
//! The task-level verdict is `fail` at
//! `"swap_between_validate_and_approve"`.
//!
//! Distinct from task-40 (TOCTOU on *tool-use* approval — world state
//! changes between approval and use) and task-92 (one binding:
//! approval→apply, lua-side): this chains *every* transition,
//! including validate→approve, which task-92 doesn't cover.
//!
//! Product decision banked for Matt: whether the pipeline should gain
//! cross-stage artifact hash chaining (each stage's input hash equals
//! the previous stage's output hash, mismatches failing closed with
//! both hashes recorded). Not implemented on gauntlet authority.

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
pub const ID: &str = "task-100";
/// Human-readable name.
pub const NAME: &str = "artifact swap between pipeline stages";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "same_artifact_all_stages",
    "record_carries_no_content_hash",
    "swap_between_validate_and_approve",
    "swap_between_approve_and_apply",
];

// ---------------------------------------------------------------------------
// Bounds and fixture constants
// ---------------------------------------------------------------------------

/// 32-hex-char candidate digest the fixture approvals cover.
const CANDIDATE_DIGEST_HEX: &str = "9f2b3c4d5e6f708192a3b4c5d6e7f809";
/// Expiry for driver fixtures (2030-01-01T00:00:00Z).
const EXPIRES_MS: u64 = 1_893_456_000_000;
/// The acting-agent identity for driver scenarios.
const DRIVER_AGENT: &str = "gauntlet-runner";
/// The enrolled operator the fixtures sign as.
const OPERATOR: &str = "gauntlet-test-operator";
/// Approval scope used by every fixture record.
const SCOPE: &str = "phlow-experiment/task-100";
/// Artifact digest the validate stage records for the BENIGN twin.
const BENIGN_ARTIFACT_DIGEST: &str = "benign9f2b";
/// Artifact digest of the MALICIOUS twin (never validated).
const MALICIOUS_ARTIFACT_DIGEST: &str = "malic10u5";
/// Maximum bytes read from one product source file.
const SOURCE_BYTES_MAX: usize = 512 * 1024;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-100 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// A recon probe failed.
    Probe {
        /// Which probe.
        case: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-100: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-100: probe {case} failed: {detail}")
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

fn probe_error(case: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Probe {
        case: case.to_string(),
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
// Fixtures (same deterministic dual-signed pattern as tasks 91/96-98)
// ---------------------------------------------------------------------------

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

fn fingerprint(keypair: &DriverKeypair) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(keypair.ed_pk);
    hasher.update(keypair.pq_pk);
    hasher.finalize().into()
}

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
    std::fs::write(&path, &toml).expect("task-100: cannot write registry fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("task-100: cannot chmod registry fixture");
    }
    OperatorRegistry::load(&path).expect("task-100: registry fixture failed to load")
}

fn scratch_dir(case: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gauntlet-task-100-{case}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("task-100: cannot create scratch dir");
    dir
}

/// An evidence bundle whose artifact digest is `artifact_digest` — the
/// "validate" stage's view of the artifact.
fn evidence_for(artifact_digest: &str) -> Result<EvidenceBundle, ExperimentError> {
    let checks = vec![CheckRun::new(
        "tests",
        vec![
            "cargo".to_string(),
            "test".to_string(),
            "--locked".to_string(),
        ],
        true,
    )?];
    let artifacts = vec![ArtifactDigest::new("candidate.diff", artifact_digest)?];
    let verification = VerificationOutcome::new(true, vec!["src/lib.rs".to_string()])?;
    EvidenceBundle::new(checks, artifacts, verification)
}

/// A minimal valid proposal touching `surface`.
fn valid_proposal(trigger: &str, surface: &str) -> Result<ImprovementProposal, ExperimentError> {
    ImprovementProposal::new(ProposalParams {
        trigger: trigger.to_string(),
        failure_category: "stage-chaining".to_string(),
        baseline_revision: "87c182d".to_string(),
        candidate_diff_summary: "Driver proposal.".to_string(),
        changed_surface: vec![surface.to_string()],
        expected_benefit: "Test stage chaining.".to_string(),
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

struct PromotionInputs {
    keypair: DriverKeypair,
    registry: OperatorRegistry,
    clock: ManualClock,
}

fn promotion_inputs(case: &str) -> PromotionInputs {
    let keypair = driver_keypair([0xa0; 32], [0x00; 32]);
    let dir = scratch_dir(case);
    let registry = test_registry(&dir, &[(OPERATOR, &keypair, false)]);
    let clock = ManualClock::new(EXPIRES_MS - 1_000);
    PromotionInputs {
        keypair,
        registry,
        clock,
    }
}

/// Workspace root: two levels above this crate's manifest directory.
fn workspace_root() -> Result<PathBuf, DriverError> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| fixture_error("workspace root", "manifest dir has no grandparent"))?;
    if !root.join("Cargo.lock").is_file() {
        return Err(fixture_error(
            "workspace root",
            format!("no Cargo.lock under {}", root.display()),
        ));
    }
    Ok(root.to_path_buf())
}

/// Assert no `fn apply` signature exists in any
/// `crates/phlow-experiment/src/*.rs` — there is no apply-stage API to
/// re-verify the artifact. Fail-closed: an apply verifier appearing
/// refutes the premise.
fn assert_no_apply_stage() -> Result<Vec<String>, DriverError> {
    const CASE: &str = "swap_between_approve_and_apply";
    let root = workspace_root()?;
    let src_dir = root.join("crates/phlow-experiment/src");
    let mut evidence = Vec::new();
    let mut files = 0usize;
    let entries = std::fs::read_dir(&src_dir)
        .map_err(|e| fixture_error("source walk", format!("{}: {e}", src_dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| fixture_error("source walk", e))?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            files += 1;
            let bytes = std::fs::read(&path)
                .map_err(|e| fixture_error("source read", format!("{}: {e}", path.display())))?;
            if bytes.len() > SOURCE_BYTES_MAX {
                return Err(probe_error(
                    CASE,
                    format!("{} exceeds {SOURCE_BYTES_MAX} bytes", path.display()),
                ));
            }
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                if line.contains("fn apply") {
                    return Err(probe_error(
                        CASE,
                        format!(
                            "an apply-stage function appeared in {} — premise changed",
                            path.display()
                        ),
                    ));
                }
            }
        }
    }
    evidence.push(format!(
        "{files} phlow-experiment source files scanned: no `fn apply` — \
         there is no apply-stage API that could re-verify the artifact"
    ));
    Ok(evidence)
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: the same (benign) artifact through propose → validate → approve
/// succeeds — the unchained pipeline works when nothing is swapped.
fn case_same_artifact_all_stages() -> Result<CaseReport, DriverError> {
    const CASE: &str = "same_artifact_all_stages";
    let mut evidence = Vec::new();
    let inputs = promotion_inputs("same-artifact");
    let record = signed_record(
        OPERATOR,
        "APR-T100-0001",
        CANDIDATE_DIGEST_HEX,
        &inputs.keypair,
    );
    let approval = HumanApproval::from_operator_record(&record)
        .map_err(|e| fixture_error("same approval", format!("record rejected: {e}")))?;
    let proposal = valid_proposal("TASK100-001", "src/approval_docs.rs")
        .map_err(|e| fixture_error("same proposal", format!("invalid: {e}")))?;
    // Validate stage: evidence built for the benign artifact.
    let bundle = evidence_for(BENIGN_ARTIFACT_DIGEST)
        .map_err(|e| fixture_error("same evidence", format!("invalid: {e}")))?;
    let mut consumed = ConsumedApprovals::new();
    match PromotionGate::promote(
        &proposal,
        approval,
        &bundle,
        DRIVER_AGENT,
        &inputs.clock,
        &inputs.registry,
        &mut consumed,
    ) {
        Ok(record_out) => {
            evidence.push(format!(
                "same artifact through all stages promoted: approval_id={}",
                record_out.approval_id()
            ));
            Ok(CaseReport::pass(
                CASE,
                serde_json::json!({"promoted": true}),
                evidence,
            ))
        }
        Err(error) => Ok(CaseReport::fail(
            CASE,
            format!("unswapped promotion rejected: {error}"),
            evidence,
        )),
    }
}

/// V2: the emitted promotion record carries the approval's candidate
/// digest (copied, uncompared) and no hash computed over the
/// proposal's content — the structural reason swaps are undetectable.
fn case_record_carries_no_content_hash() -> Result<CaseReport, DriverError> {
    const CASE: &str = "record_carries_no_content_hash";
    let mut evidence = Vec::new();
    let inputs = promotion_inputs("record-fields");
    let record = signed_record(
        OPERATOR,
        "APR-T100-0002",
        CANDIDATE_DIGEST_HEX,
        &inputs.keypair,
    );
    let approval = HumanApproval::from_operator_record(&record)
        .map_err(|e| fixture_error("record approval", format!("record rejected: {e}")))?;
    let approval_digest = approval.candidate_digest().to_string();
    let proposal = valid_proposal("TASK100-002", "src/approval_docs.rs")
        .map_err(|e| fixture_error("record proposal", format!("invalid: {e}")))?;
    let bundle = evidence_for(BENIGN_ARTIFACT_DIGEST)
        .map_err(|e| fixture_error("record evidence", format!("invalid: {e}")))?;
    let mut consumed = ConsumedApprovals::new();
    let record_out = PromotionGate::promote(
        &proposal,
        approval,
        &bundle,
        DRIVER_AGENT,
        &inputs.clock,
        &inputs.registry,
        &mut consumed,
    )
    .map_err(|e| fixture_error("record promotion", format!("unexpected rejection: {e}")))?;
    // The record's digest is the approval's digest verbatim — copied,
    // never computed from the proposal.
    if record_out.candidate_digest() != approval_digest {
        return Ok(CaseReport::fail(
            CASE,
            "record digest differs from the approval digest (premise changed)".to_string(),
            evidence,
        ));
    }
    evidence.push(format!(
        "record.candidate_digest == approval.candidate_digest ({approval_digest}): copied, uncompared"
    ));
    evidence.push(format!(
        "record fields: candidate_digest, baseline_revision={}, rollback_target={}, \
         operator={}, approval_id={}, evidence_checks={} — none is a content hash of the proposal",
        "87c182d",
        record_out.rollback_target(),
        record_out.operator(),
        record_out.approval_id(),
        record_out.evidence_checks(),
    ));
    evidence.push(
        "ImprovementProposal exposes no digest/content-hash accessor and \
         EvidenceBundle's artifact digests are never compared to the proposal \
         inside promote()"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"record_has_content_hash": false}),
        evidence,
    ))
}

/// A1: swap between validate and approve — the evidence bundle is
/// built for the BENIGN twin while the proposal promoted is the
/// MALICIOUS twin. The promotion SUCCEEDS: no stage binds the
/// validated artifact to the approved proposal.
fn case_swap_between_validate_and_approve() -> Result<CaseReport, DriverError> {
    const CASE: &str = "swap_between_validate_and_approve";
    let mut evidence = Vec::new();
    let inputs = promotion_inputs("swap-validate-approve");
    let record = signed_record(
        OPERATOR,
        "APR-T100-0003",
        CANDIDATE_DIGEST_HEX,
        &inputs.keypair,
    );
    let approval = HumanApproval::from_operator_record(&record)
        .map_err(|e| fixture_error("swap approval", format!("record rejected: {e}")))?;
    // Validate stage saw the BENIGN twin ...
    let bundle = evidence_for(BENIGN_ARTIFACT_DIGEST)
        .map_err(|e| fixture_error("swap evidence", format!("invalid: {e}")))?;
    evidence.push(format!(
        "validate stage: evidence built for benign artifact digest {BENIGN_ARTIFACT_DIGEST}"
    ));
    // ... but the approve stage is handed the MALICIOUS twin.
    let malicious = valid_proposal("TASK100-003", "src/evil.rs")
        .map_err(|e| fixture_error("malicious proposal", format!("invalid: {e}")))?;
    evidence.push(format!(
        "approve stage: proposal is the malicious twin (surface src/evil.rs, \
         artifact digest {MALICIOUS_ARTIFACT_DIGEST} — never validated)"
    ));
    let mut consumed = ConsumedApprovals::new();
    match PromotionGate::promote(
        &malicious,
        approval,
        &bundle,
        DRIVER_AGENT,
        &inputs.clock,
        &inputs.registry,
        &mut consumed,
    ) {
        Ok(record_out) => {
            evidence.push(format!(
                "SWAPPED AND PROMOTED: benign-twin evidence + malicious-twin proposal \
                 promoted (approval_id={}); no stage_artifact_mismatch exists",
                record_out.approval_id(),
            ));
            Ok(CaseReport::fail(
                CASE,
                "swap_between_validate_and_approve: the validated-twin swap succeeded — the \
                 evidence bundle validated the benign artifact while the gate approved the \
                 malicious proposal, and no stage compared the two. Detection is by nothing: \
                 there is no content-hash chaining between validate and approve"
                    .to_string(),
                evidence,
            ))
        }
        Err(error) => {
            evidence.push(format!("swapped promotion rejected: {error}"));
            Ok(CaseReport::pass(
                CASE,
                serde_json::json!({"swap_rejected": true}),
                evidence,
            ))
        }
    }
}

/// A2: swap between approve and apply — after a successful promotion,
/// the human-driven apply has no in-band re-verification: no
/// apply-stage API exists, so a swapped artifact (even one preserving
/// the filename but not the bytes) is undetectable in-band.
fn case_swap_between_approve_and_apply() -> Result<CaseReport, DriverError> {
    const CASE: &str = "swap_between_approve_and_apply";
    let mut evidence = assert_no_apply_stage().map_err(|e| match e {
        DriverError::Probe { detail, .. } => DriverError::Probe {
            case: CASE.to_string(),
            detail,
        },
        other => other,
    })?;
    let inputs = promotion_inputs("swap-approve-apply");
    let record = signed_record(
        OPERATOR,
        "APR-T100-0004",
        CANDIDATE_DIGEST_HEX,
        &inputs.keypair,
    );
    let approval = HumanApproval::from_operator_record(&record)
        .map_err(|e| fixture_error("apply approval", format!("record rejected: {e}")))?;
    let proposal = valid_proposal("TASK100-004", "src/approval_docs.rs")
        .map_err(|e| fixture_error("apply proposal", format!("invalid: {e}")))?;
    let bundle = evidence_for(BENIGN_ARTIFACT_DIGEST)
        .map_err(|e| fixture_error("apply evidence", format!("invalid: {e}")))?;
    let mut consumed = ConsumedApprovals::new();
    let record_out = PromotionGate::promote(
        &proposal,
        approval,
        &bundle,
        DRIVER_AGENT,
        &inputs.clock,
        &inputs.registry,
        &mut consumed,
    )
    .map_err(|e| fixture_error("apply promotion", format!("unexpected rejection: {e}")))?;
    evidence.push(format!(
        "approve stage emitted PromotionRecord (approval_id={}); apply is human-driven \
         merge by design — no code re-hashes the artifact between the record and the merge",
        record_out.approval_id(),
    ));
    evidence.push(
        "a post-approval artifact swap (same filename, different bytes) has no in-band \
         tripwire: the record carries no content hash, and there is no apply-stage verifier"
            .to_string(),
    );
    Ok(CaseReport::fail(
        CASE,
        "swap_between_approve_and_apply: the approve→apply transition has no artifact \
         verification — apply is human-driven merge with no re-hash, the promotion record \
         carries no content hash of the approved artifact, and no apply-stage API exists. \
         A swapped artifact between approval and merge is undetectable in-band"
            .to_string(),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "same_artifact_all_stages" => case_same_artifact_all_stages(),
        "record_carries_no_content_hash" => case_record_carries_no_content_hash(),
        "swap_between_validate_and_approve" => case_swap_between_validate_and_approve(),
        "swap_between_approve_and_apply" => case_swap_between_approve_and_apply(),
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
        "seam: ABSENT — propose/validate/approve take independent inputs; \
         no content-hash chaining across any stage transition; no apply \
         stage exists in code"
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
                " Product decision banked for Matt: whether the pipeline should gain \
                 cross-stage artifact hash chaining (each stage's input hash equals the \
                 previous stage's output hash, mismatches failing closed with both hashes \
                 recorded) is not implemented on gauntlet authority.",
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
