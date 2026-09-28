//! Integration tests for task-11 (self-approval rejected).
//!
//! 50/50 split: 2 validation, 2 adversarial. Every test drives the REAL
//! phlow-experiment APIs (`HumanApproval`, `PromotionGate` with the
//! 7-argument call-site shape, `OperatorRegistry`, `ManualClock`) — no
//! mocks, no test doubles. Fixtures are genuine v2 operator records:
//! deterministic Ed25519 + ML-DSA-65 keypairs (fixed test seeds, never
//! real key material) dual-sign the records in the test itself, and a
//! real TOML registry pins the keys.

use phlow_experiment::{
    ArtifactDigest, CheckRun, ConsumedApprovals, EvidenceBundle, ExperimentError, HumanApproval,
    ImprovementProposal, ManualClock, OperatorRegistry, PromotionGate, ProposalBudgets,
    ProposalParams, ReviewDecision, ReviewerDecision, RiskClass, VerificationOutcome, WorkerRole,
};
use phlow_gauntlet::tasks::task_11;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::{Path, PathBuf};

/// The `signature::Keypair` trait (re-exported by `ml-dsa`), needed for
/// `verifying_key()` on the ML-DSA signing key.
use ml_dsa::Keypair as _;

/// 32-hex-char candidate digest used by every fixture.
const CANDIDATE_DIGEST_HEX: &str = "9f2b3c4d5e6f708192a3b4c5d6e7f809";
/// Expiry for fixtures (2030-01-01T00:00:00Z); the clock sits just before it.
const EXPIRES_MS: u64 = 1_893_456_000_000;
/// Acting agent that is never an enrolled operator.
const DRIVER_AGENT: &str = "gauntlet-runner";

/// Unwrap a `Result`, panicking with the typed domain error on failure.
fn ok<T>(result: Result<T, ExperimentError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("expected Ok, got error: {error}"),
    }
}

/// A deterministic Ed25519 + ML-DSA-65 keypair. Fixed test seeds — never
/// real key material — so fixtures are reproducible without randomness.
struct TestKeypair {
    ed_sk: ed25519_dalek::SigningKey,
    pq_sk: ml_dsa::SigningKey<ml_dsa::MlDsa65>,
    ed_pk: [u8; 32],
    pq_pk: [u8; 1952],
}

fn keypair(ed_seed: [u8; 32], pq_seed: [u8; 32]) -> TestKeypair {
    let ed_sk = ed25519_dalek::SigningKey::from_bytes(&ed_seed);
    let seed = ml_dsa::Seed::from(pq_seed);
    let pq_sk = ml_dsa::SigningKey::<ml_dsa::MlDsa65>::from_seed(&seed);
    let ed_pk = ed_sk.verifying_key().to_bytes();
    let mut pq_pk = [0u8; 1952];
    pq_pk.copy_from_slice(pq_sk.verifying_key().encode().as_slice());
    TestKeypair {
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

/// Enrollment fingerprint: SHA-256 over `ed_pk || pq_pk`.
fn fingerprint(keypair: &TestKeypair) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(keypair.ed_pk);
    hasher.update(keypair.pq_pk);
    hasher.finalize().into()
}

/// A genuine v2 operator record: Ed25519 over the canonical bytes,
/// ML-DSA-65 over `canonical || ed_sig`.
fn signed_record(operator: &str, approval_id: &str, keypair: &TestKeypair) -> String {
    use ml_dsa::Signer as _;
    let canonical = format!(
        "v: 2\n\
         operator: {operator}\n\
         approval_id: {approval_id}\n\
         candidate: {CANDIDATE_DIGEST_HEX}\n\
         scope: phlow-experiment/task-11\n\
         expires_ms: {EXPIRES_MS}"
    )
    .into_bytes();
    let ed_sig = keypair.ed_sk.sign(&canonical);
    let mut pq_message = canonical;
    pq_message.extend_from_slice(&ed_sig.to_bytes());
    let pq_sig = keypair.pq_sk.sign(&pq_message);
    format!(
        "v: 2\n\
         operator: {operator}\n\
         approval_id: {approval_id}\n\
         candidate: {CANDIDATE_DIGEST_HEX}\n\
         scope: phlow-experiment/task-11\n\
         expires_ms: {EXPIRES_MS}\n\
         signature_ed25519: {}\n\
         signature_mldsa65: {}\n",
        hex(&ed_sig.to_bytes()),
        hex(pq_sig.encode().as_slice()),
    )
}

/// A fresh scratch dir for one test's registry file.
fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gauntlet-task-11-test-{test}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("cannot create scratch dir");
    dir
}

/// Writes a real TOML operator registry under `dir` and loads it through
/// the real loader. Entries: (name, keypair, revoked).
fn registry_with(dir: &Path, entries: &[(&str, &TestKeypair, bool)]) -> OperatorRegistry {
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
    std::fs::write(&path, &toml).expect("cannot write registry fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("cannot chmod registry fixture");
    }
    ok(OperatorRegistry::load(&path))
}

/// A complete evidence bundle: one passing check with exact argv, one
/// artifact digest, one verified outcome with coverage.
fn complete_evidence() -> EvidenceBundle {
    let checks = vec![ok(CheckRun::new(
        "tests",
        vec![
            "cargo".to_string(),
            "test".to_string(),
            "--locked".to_string(),
        ],
        true,
    ))];
    let artifacts = vec![ok(ArtifactDigest::new("candidate.diff", "9f2b3c4d"))];
    let verification = ok(VerificationOutcome::new(
        true,
        vec!["src/lib.rs".to_string()],
    ));
    ok(EvidenceBundle::new(checks, artifacts, verification))
}

/// A minimal valid proposal: one unprotected file, one approving reviewer.
fn valid_proposal() -> ImprovementProposal {
    ok(ImprovementProposal::new(ProposalParams {
        trigger: "TASK11-001".to_string(),
        failure_category: "self-approval".to_string(),
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
    }))
}

/// The 7-argument call-site shape: proposal, approval, evidence, acting
/// agent, trusted clock, operator registry, replay store.
fn promote(
    proposal: &ImprovementProposal,
    record: &str,
    acting_agent: &str,
    clock: &ManualClock,
    registry: &OperatorRegistry,
    consumed: &mut ConsumedApprovals,
) -> Result<(), ExperimentError> {
    let approval = HumanApproval::from_operator_record(record)?;
    PromotionGate::promote(
        proposal,
        approval,
        &complete_evidence(),
        acting_agent,
        clock,
        registry,
        consumed,
    )?;
    Ok(())
}

/// A `Ctx` whose work dir is fresh under the system temp dir. The nvim
/// paths are unused by this Rust-kind task; `Ctx::new` only rejects empty
/// paths, so placeholder values are fine.
fn ctx_for(test: &str) -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-11-{test}-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("/nonexistent/nvim"),
        PathBuf::from("/nonexistent/diver-lua"),
        work_dir,
    )
    .unwrap_or_else(|error| panic!("task-11: cannot build Ctx: {error}"))
}

#[test]
fn task_metadata_intact() {
    assert_eq!(task_11::ID, "task-11");
    assert_eq!(task_11::NAME, "self-approval rejected");
    assert_eq!(task_11::KIND, TaskKind::Rust);
}

/// V1 (control): a genuine v2 operator record — dual-signed with the
/// enrolled operator's real keys — parses, verifies against the registry,
/// and promotes end to end through the real gate with a distinct acting
/// agent, a live clock, and a fresh replay store.
#[test]
fn genuine_operator_approval_promotes() {
    let keypair = keypair([0x42; 32], [0x24; 32]);
    let dir = scratch("genuine");
    let registry = registry_with(&dir, &[("gauntlet-test-operator", &keypair, false)]);
    let record = signed_record("gauntlet-test-operator", "APR-T11-TEST-0001", &keypair);
    let clock = ManualClock::new(EXPIRES_MS - 1_000);
    let mut consumed = ConsumedApprovals::new();
    promote(
        &valid_proposal(),
        &record,
        DRIVER_AGENT,
        &clock,
        &registry,
        &mut consumed,
    )
    .expect("genuine dual-signed approval must promote");
    assert!(task_11::scenario_genuine_operator_approval().requirement_met);
}

/// V2: an agent that tries to approve its own action with no operator
/// record — bare model output — is rejected at every attempt, so no
/// `HumanApproval` value ever exists for the gate to consume.
#[test]
fn self_approval_without_record_rejected() {
    for record in [
        "",
        "I approve this action",
        "{\"approved\": true, \"by\": \"agent-007\"}",
        "operator: agent-007\napproved: yes\n",
    ] {
        assert!(
            matches!(
                HumanApproval::from_operator_record(record),
                Err(ExperimentError::ApprovalRejected { .. })
            ),
            "self-approval without a valid operator record was accepted: {record:?}"
        );
    }
    assert!(task_11::scenario_self_approval_without_record().requirement_met);
}

/// A1: tampered records are rejected at parse — and now a well-shaped
/// record with an attacker-minted signature is rejected at the gate with
/// `BadSignature`: the dual signature is verified against the
/// registry-pinned keys, and the attacker's bytes match neither half.
#[test]
fn forged_record_rejected_at_gate() {
    let keypair = keypair([0x42; 32], [0x24; 32]);
    let dir = scratch("forged");
    let registry = registry_with(&dir, &[("gauntlet-test-operator", &keypair, false)]);
    let base = signed_record("gauntlet-test-operator", "APR-T11-TEST-0002", &keypair);
    // Well-shaped forgery: every key present, every value well-formed,
    // both signatures attacker-minted hex of the right length. It parses —
    // shape validation is not authentication.
    let mut forged = String::new();
    for line in base.lines() {
        if line.starts_with("signature_ed25519:") {
            forged.push_str(&format!("signature_ed25519: {}\n", "cd".repeat(64)));
        } else if line.starts_with("signature_mldsa65:") {
            forged.push_str(&format!("signature_mldsa65: {}\n", "ef".repeat(3309)));
        } else {
            forged.push_str(line);
            forged.push('\n');
        }
    }
    HumanApproval::from_operator_record(&forged)
        .expect("well-shaped forgery must parse (shape is not auth)");
    // But the gate rejects it with BadSignature.
    let clock = ManualClock::new(EXPIRES_MS - 1_000);
    let mut consumed = ConsumedApprovals::new();
    let error = promote(
        &valid_proposal(),
        &forged,
        DRIVER_AGENT,
        &clock,
        &registry,
        &mut consumed,
    )
    .expect_err("attacker-minted signatures must not promote");
    assert!(
        matches!(error, ExperimentError::BadSignature { .. }),
        "expected BadSignature, got: {error}"
    );
    // Tampered variants: rejected at parse.
    let mut tampered: Vec<String> = Vec::new();
    tampered.push(format!("{base}operator: gauntlet-test-operator\n"));
    tampered.push(format!("{base}injected: true\n"));
    let mut non_hex = String::new();
    for line in base.lines() {
        if line.starts_with("signature_ed25519:") {
            non_hex.push_str("signature_ed25519: ");
            non_hex.push_str(&"z".repeat(128));
            non_hex.push('\n');
        } else {
            non_hex.push_str(line);
            non_hex.push('\n');
        }
    }
    tampered.push(non_hex);
    tampered.push(base.replace("expires_ms: 1893456000000", "expires_ms: 0"));
    tampered.push(base.replace(CANDIDATE_DIGEST_HEX, "zzzz"));
    for record in &tampered {
        assert!(
            HumanApproval::from_operator_record(record).is_err(),
            "tampered record accepted: {record:?}"
        );
    }
    assert!(task_11::scenario_forged_record().requirement_met);
}

/// A2: confused identity — the agent mints a genuine v2 record naming
/// ITSELF as the operator for its own candidate, dual-signed with its own
/// enrolled keys. The gate still rejects it with `SelfApproval`: the
/// acting-agent identity is the first check, before any cryptography.
#[test]
fn confused_identity_rejected() {
    let keypair = keypair([0x46; 32], [0x28; 32]);
    let dir = scratch("confused");
    let registry = registry_with(&dir, &[("agent-007", &keypair, false)]);
    let record = signed_record("agent-007", "APR-T11-TEST-0007", &keypair);
    let clock = ManualClock::new(EXPIRES_MS - 1_000);
    let mut consumed = ConsumedApprovals::new();
    let error = promote(
        &valid_proposal(),
        &record,
        "agent-007",
        &clock,
        &registry,
        &mut consumed,
    )
    .expect_err("self-approval must not promote");
    assert!(
        matches!(error, ExperimentError::SelfApproval { .. }),
        "expected SelfApproval, got: {error}"
    );
    // The same record promotes for a different acting agent: the check is
    // precise, not a blanket block.
    let mut consumed = ConsumedApprovals::new();
    promote(
        &valid_proposal(),
        &record,
        DRIVER_AGENT,
        &clock,
        &registry,
        &mut consumed,
    )
    .expect("other-agent promotion must succeed");
    assert!(task_11::scenario_confused_identity().requirement_met);
}

/// The driver aggregates honestly: all four security requirements are
/// met, so it reports Pass with evidence and writes its report under the
/// task work dir.
#[test]
fn driver_reports_pass_with_evidence() {
    let ctx = ctx_for("driver");
    let outcome = task_11::run(&ctx);
    match &outcome {
        TaskOutcome::Pass { evidence } => {
            assert!(
                evidence.len() >= 4,
                "pass evidence should list all scenarios, got: {evidence:?}"
            );
        }
        TaskOutcome::Fail { where_, how, .. } => {
            panic!("driver reported failure: {where_}: {how}");
        }
    }
    let report = ctx.work_dir.join("task-11").join("report.md");
    assert!(
        report.is_file(),
        "driver must write its report, missing: {}",
        report.display()
    );
}
