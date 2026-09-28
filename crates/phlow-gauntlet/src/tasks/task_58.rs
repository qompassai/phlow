//! task-58: dual control (rust).
//!
//! Drives phlow-experiment's real approval seam — [`HumanApproval`],
//! [`PromotionGate::promote`], `OperatorRegistry`, `ManualClock` —
//! through four scenarios (2 validation, 2 adversarial) against the
//! design's dual-control requirement: a high-risk action requires two
//! DISTINCT human approvers; the gate counts distinct validated
//! operator identities; any shortfall is rejected naming the shortfall.
//!
//! Verified against the source (`crates/phlow-experiment/src/
//! promotion.rs`): [`PromotionGate::promote`] takes exactly ONE
//! [`HumanApproval`]. There is no quorum parameter, no second-approval
//! slot, no distinct-identity check across approvals, and no
//! quorum/dual-control vocabulary anywhere in phlow-experiment. The
//! approval record DOES bind a candidate digest (signed into the
//! canonical bytes), but `promote` never compares
//! `approval.candidate_digest()` to the proposal — the check order in
//! the gate's docs lists every check, and the candidate binding is not
//! among them.
//!
//! Honest result: FAIL at the seam. A single genuine operator approval
//! promotes (V1, the control); two distinct operators' approvals each
//! promote independently with no single decision requiring both (V2);
//! the same operator approving twice is never distinguished (A1); and
//! changing the proposal's parameters after an approval requires no
//! second approval because the bound candidate digest is never verified
//! against the proposal (A2).
//!
//! Banked for Matt (product decision, NOT auto-implemented on gauntlet
//! authority): whether the promotion gate should gain a
//! two-distinct-operator quorum for high-risk actions — a new API
//! (quorum parameter, cross-approval distinctness on validated operator
//! identity, both approvals bound to the same action hash, agent-issued
//! approvals never counting toward the quorum).

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{
    ArtifactDigest, CheckRun, ConsumedApprovals, EvidenceBundle, ExperimentError, HumanApproval,
    ImprovementProposal, ManualClock, OperatorRegistry, PromotionGate, ProposalBudgets,
    ProposalParams, ReviewDecision, ReviewerDecision, RiskClass, VerificationOutcome, WorkerRole,
};
use std::path::{Path, PathBuf};

/// The `signature::Keypair` trait (re-exported by `ml-dsa`), needed for
/// `verifying_key()` on the ML-DSA signing key.
use ml_dsa::Keypair as _;

/// Task id.
pub const ID: &str = "task-58";
/// Human-readable name.
pub const NAME: &str = "dual control";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Maximum evidence lines kept per scenario; [`run`] caps the total at
/// the crate's `EVIDENCE_LINES_MAX` via [`bound_evidence`].
const SCENARIO_EVIDENCE_LINES_MAX: usize = 12;

/// 32-hex-char candidate digest bound into every fixture record.
const CANDIDATE_DIGEST_HEX: &str = "9f2b3c4d5e6f708192a3b4c5d6e7f809";

/// Expiry for driver fixtures (2030-01-01T00:00:00Z).
const EXPIRES_MS: u64 = 1_893_456_000_000;

/// The acting-agent identity for driver scenarios. It is deliberately not
/// an enrolled operator.
const DRIVER_AGENT: &str = "gauntlet-runner";

/// The observed result of one scenario.
#[derive(Debug, Clone)]
pub struct ScenarioVerdict {
    /// Scenario name, e.g. `"single-genuine-approval-promotes"`.
    pub name: &'static str,
    /// True for validation scenarios, false for adversarial ones.
    pub validation: bool,
    /// True when the observed behavior satisfies the dual-control
    /// requirement.
    pub requirement_met: bool,
    /// Bounded evidence lines (at most [`SCENARIO_EVIDENCE_LINES_MAX`]).
    pub evidence: Vec<String>,
}

/// Builds a [`ScenarioVerdict`] with bounded evidence.
fn verdict(
    name: &'static str,
    validation: bool,
    requirement_met: bool,
    evidence: Vec<String>,
) -> ScenarioVerdict {
    let mut evidence = evidence;
    evidence.truncate(SCENARIO_EVIDENCE_LINES_MAX);
    ScenarioVerdict {
        name,
        validation,
        requirement_met,
        evidence,
    }
}

// ---------------------------------------------------------------------------
// Fixtures: real dual-signed v2 records, generated in-driver.
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
fn canonical_bytes(operator: &str, approval_id: &str, expires_ms: u64) -> Vec<u8> {
    format!(
        "v: 2\n\
         operator: {operator}\n\
         approval_id: {approval_id}\n\
         candidate: {CANDIDATE_DIGEST_HEX}\n\
         scope: phlow-experiment/task-58\n\
         expires_ms: {expires_ms}"
    )
    .into_bytes()
}

/// Builds a v2 operator record dual-signed with `keypair`: Ed25519 over
/// the canonical bytes, ML-DSA-65 over `canonical || ed_sig`.
fn signed_record(operator: &str, approval_id: &str, keypair: &DriverKeypair) -> String {
    use ml_dsa::Signer as _;
    let canonical = canonical_bytes(operator, approval_id, EXPIRES_MS);
    let ed_sig = keypair.ed_sk.sign(&canonical);
    let mut pq_message = canonical;
    pq_message.extend_from_slice(&ed_sig.to_bytes());
    let pq_sig = keypair.pq_sk.sign(&pq_message);
    format!(
        "v: 2\n\
         operator: {operator}\n\
         approval_id: {approval_id}\n\
         candidate: {CANDIDATE_DIGEST_HEX}\n\
         scope: phlow-experiment/task-58\n\
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
    std::fs::write(&path, &toml).expect("task-58: cannot write registry fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("task-58: cannot chmod registry fixture");
    }
    OperatorRegistry::load(&path).expect("task-58: registry fixture failed to load")
}

/// A fresh scratch dir for one scenario's registry file.
fn scratch_dir(scenario: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gauntlet-task-58-{scenario}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("task-58: cannot create scratch dir");
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

/// A minimal valid proposal with the given baseline revision and changed
/// surface: one unprotected changed file, one approving reviewer,
/// positive budgets.
fn proposal_for(
    baseline_revision: &str,
    changed: &str,
) -> Result<ImprovementProposal, ExperimentError> {
    ImprovementProposal::new(ProposalParams {
        trigger: "TASK58-001".to_string(),
        failure_category: "dual-control".to_string(),
        baseline_revision: baseline_revision.to_string(),
        candidate_diff_summary: "Harden the dual-control seam.".to_string(),
        changed_surface: vec![changed.to_string()],
        expected_benefit: "Two-operator promotion policy.".to_string(),
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
        rollback_target: baseline_revision.to_string(),
    })
}

/// Promotes `proposal` with `approval` against fresh fixtures and
/// returns the resulting [`PromotionRecord`]'s operator on success.
fn promote_once(
    proposal: &ImprovementProposal,
    record_text: &str,
    registry: &OperatorRegistry,
) -> Result<String, ExperimentError> {
    let approval = HumanApproval::from_operator_record(record_text)?;
    let bundle = complete_evidence()?;
    let clock = ManualClock::new(EXPIRES_MS - 1_000);
    let mut consumed = ConsumedApprovals::new();
    let promotion = PromotionGate::promote(
        proposal,
        approval,
        &bundle,
        DRIVER_AGENT,
        &clock,
        registry,
        &mut consumed,
    )?;
    Ok(promotion.operator().to_string())
}

/// V1 (control): a single genuine v2 operator record promotes. This is
/// the gate's real one-approval contract — the baseline the dual-control
/// requirement would have to strengthen.
pub fn scenario_single_genuine_approval_promotes() -> ScenarioVerdict {
    let mut evidence = Vec::new();
    let requirement_met = match run_single_genuine_approval_promotes(&mut evidence) {
        Ok(()) => true,
        Err(reason) => {
            evidence.push(format!("control FAILED: {reason}"));
            false
        }
    };
    verdict(
        "single-genuine-approval-promotes",
        true,
        requirement_met,
        evidence,
    )
}

fn run_single_genuine_approval_promotes(evidence: &mut Vec<String>) -> Result<(), String> {
    let keypair = driver_keypair([0x58; 32], [0x85; 32]);
    let dir = scratch_dir("single");
    let registry = test_registry(&dir, &[("operator-a", &keypair, false)]);
    let record = signed_record("operator-a", "APR-T58-0001", &keypair);
    let proposal = proposal_for("87c182d", "src/dual_control_docs.rs")
        .map_err(|error| format!("fixture proposal invalid: {error}"))?;
    let operator = promote_once(&proposal, &record, &registry)
        .map_err(|error| format!("genuine single approval rejected: {error}"))?;
    if operator != "operator-a" {
        return Err("PromotionRecord.operator() mismatch".to_string());
    }
    evidence.push(
        "one genuine dual-signed v2 record (operator-a) promotes: \
         PromotionGate::promote -> Ok. The gate's contract is one \
         approval per promotion — dual control would have to add a \
         second-approval requirement on top of this"
            .to_string(),
    );
    Ok(())
}

/// V2: two DISTINCT enrolled operators approve. The design's default
/// scenario wants ONE gate decision requiring both. Reality: each
/// approval promotes in its own independent call — two PromotionRecords,
/// two consumed ids, no single decision that required both, no
/// cross-approval distinctness check. (The agent-issued half of the
/// design's A2 is folded in here: an agent-issued record for the agent's
/// own action is rejected with SelfApproval — task-11's seam — so a
/// "human + agent" quorum never forms either; the human approval alone
/// suffices.)
pub fn scenario_two_distinct_operators_no_quorum() -> ScenarioVerdict {
    let mut evidence = Vec::new();
    let requirement_met = match run_two_distinct_operators_no_quorum(&mut evidence) {
        Ok(()) => true,
        Err(reason) => {
            evidence.push(format!("FAILED: {reason}"));
            false
        }
    };
    verdict(
        "two-distinct-operators-no-quorum",
        true,
        requirement_met,
        evidence,
    )
}

fn run_two_distinct_operators_no_quorum(evidence: &mut Vec<String>) -> Result<(), String> {
    let keypair_a = driver_keypair([0x5a; 32], [0xa5; 32]);
    let keypair_b = driver_keypair([0x5b; 32], [0xb5; 32]);
    let dir = scratch_dir("two-operators");
    let registry = test_registry(
        &dir,
        &[
            ("operator-a", &keypair_a, false),
            ("operator-b", &keypair_b, false),
        ],
    );
    let record_a = signed_record("operator-a", "APR-T58-0002", &keypair_a);
    let record_b = signed_record("operator-b", "APR-T58-0003", &keypair_b);
    let proposal = proposal_for("87c182d", "src/dual_control_docs.rs")
        .map_err(|error| format!("fixture proposal invalid: {error}"))?;
    let operator_a = promote_once(&proposal, &record_a, &registry)
        .map_err(|error| format!("operator-a approval rejected: {error}"))?;
    let operator_b = promote_once(&proposal, &record_b, &registry)
        .map_err(|error| format!("operator-b approval rejected: {error}"))?;
    if operator_a != "operator-a" || operator_b != "operator-b" {
        return Err("PromotionRecord.operator() mismatch".to_string());
    }
    evidence.push(
        "two distinct enrolled operators (operator-a, operator-b) each hold a \
         genuine dual-signed approval; each promotes INDEPENDENTLY — two \
         PromotionRecords, two consumed approval ids, no single gate decision \
         that required both"
            .to_string(),
    );
    evidence.push(
        "promote's call-site shape takes exactly one HumanApproval \
         (proposal, approval, evidence, acting_agent, clock, registry, \
         consumed): there is no quorum parameter and no second-approval slot, \
         so distinctness across approvals cannot be enforced"
            .to_string(),
    );
    // The design's "one human + the agent's own approval" arm: the agent's
    // record for its own action is rejected with SelfApproval (task-11's
    // seam, re-verified minimally), and the human approval above already
    // promoted alone — no quorum ever forms.
    let keypair_agent = driver_keypair([0x5c; 32], [0xc5; 32]);
    let dir_agent = scratch_dir("two-operators-agent");
    let registry_agent = test_registry(&dir_agent, &[("agent-007", &keypair_agent, false)]);
    let agent_record = signed_record("agent-007", "APR-T58-0004", &keypair_agent);
    let approval = HumanApproval::from_operator_record(&agent_record)
        .map_err(|error| format!("agent record rejected at parse: {error}"))?;
    let bundle =
        complete_evidence().map_err(|error| format!("fixture evidence invalid: {error}"))?;
    let clock = ManualClock::new(EXPIRES_MS - 1_000);
    let mut consumed = ConsumedApprovals::new();
    match PromotionGate::promote(
        &proposal,
        approval,
        &bundle,
        "agent-007",
        &clock,
        &registry_agent,
        &mut consumed,
    ) {
        Err(ExperimentError::SelfApproval { .. }) => evidence.push(
            "agent-issued approval for the agent's own action rejected with \
             SelfApproval: agent approvals never reach a quorum — but no \
             quorum exists to reach, and the human approval above promoted \
             alone"
                .to_string(),
        ),
        other => {
            return Err(format!(
                "agent self-approval was not rejected with SelfApproval: {other:?}"
            ));
        }
    }
    // The dual-control requirement — one decision requiring two distinct
    // validated operator identities — is unmet: single approvals promote.
    Err(
        "dual control unmet: two distinct operators' approvals promote \
         independently; no gate decision requires both"
            .to_string(),
    )
}

/// A1: the SAME operator approves twice (two genuine records, distinct
/// ids). The design wants this rejected on identity distinctness.
/// Reality: each promotes independently — the gate never sees two
/// approvals at once, so distinctness cannot be checked.
pub fn scenario_same_operator_twice_not_distinguished() -> ScenarioVerdict {
    let mut evidence = Vec::new();
    let requirement_met = match run_same_operator_twice_not_distinguished(&mut evidence) {
        Ok(()) => true,
        Err(reason) => {
            evidence.push(format!("FAILED: {reason}"));
            false
        }
    };
    verdict(
        "same-operator-twice-not-distinguished",
        false,
        requirement_met,
        evidence,
    )
}

fn run_same_operator_twice_not_distinguished(evidence: &mut Vec<String>) -> Result<(), String> {
    let keypair = driver_keypair([0x5d; 32], [0xd5; 32]);
    let dir = scratch_dir("same-operator");
    let registry = test_registry(&dir, &[("operator-a", &keypair, false)]);
    let record_first = signed_record("operator-a", "APR-T58-0005", &keypair);
    let record_second = signed_record("operator-a", "APR-T58-0006", &keypair);
    let proposal = proposal_for("87c182d", "src/dual_control_docs.rs")
        .map_err(|error| format!("fixture proposal invalid: {error}"))?;
    let first = promote_once(&proposal, &record_first, &registry)
        .map_err(|error| format!("first approval rejected: {error}"))?;
    let second = promote_once(&proposal, &record_second, &registry)
        .map_err(|error| format!("second approval rejected: {error}"))?;
    if first != "operator-a" || second != "operator-a" {
        return Err("PromotionRecord.operator() mismatch".to_string());
    }
    evidence.push(
        "the same operator (operator-a) issued two genuine approvals with \
         distinct ids; BOTH promote independently — the gate consumed each \
         approval by value in its own call and never compared operator \
         identities across approvals"
            .to_string(),
    );
    Err(
        "dual control unmet: same-operator-twice is not distinguished — \
         no identity-distinctness check exists because the gate never holds \
         two approvals at once"
            .to_string(),
    )
}

/// A2: one approval is issued, then the action's parameters change. The
/// design wants a second approval. The record DOES bind a candidate
/// digest (signed into the canonical bytes), but `promote` never
/// compares `approval.candidate_digest()` to the proposal — the gate's
/// documented check order (acting agent, replay, registry, TTL, expiry,
/// signatures, evidence, surface, reviewers) has no candidate-binding
/// check. A proposal with different parameters promotes with an
/// equivalent approval: no second approval is required.
pub fn scenario_params_change_needs_no_second_approval() -> ScenarioVerdict {
    let mut evidence = Vec::new();
    let requirement_met = match run_params_change_needs_no_second_approval(&mut evidence) {
        Ok(()) => true,
        Err(reason) => {
            evidence.push(format!("FAILED: {reason}"));
            false
        }
    };
    verdict(
        "params-change-needs-no-second-approval",
        false,
        requirement_met,
        evidence,
    )
}

fn run_params_change_needs_no_second_approval(evidence: &mut Vec<String>) -> Result<(), String> {
    let keypair = driver_keypair([0x5e; 32], [0xe5; 32]);
    let dir = scratch_dir("params-change");
    let registry = test_registry(&dir, &[("operator-a", &keypair, false)]);
    let record_before = signed_record("operator-a", "APR-T58-0007", &keypair);
    let record_after = signed_record("operator-a", "APR-T58-0008", &keypair);
    // The approval really does bind the candidate digest: the accessor
    // exposes it and the signature covers the canonical bytes.
    let approval = HumanApproval::from_operator_record(&record_before)
        .map_err(|error| format!("approval rejected at parse: {error}"))?;
    if approval.candidate_digest() != CANDIDATE_DIGEST_HEX {
        return Err("approval.candidate_digest() mismatch".to_string());
    }
    evidence.push(
        "the record binds candidate digest 9f2b3c4d...f809 (signed into the \
         canonical bytes; exposed via approval.candidate_digest())"
            .to_string(),
    );
    // Proposal P1 promotes with the first approval.
    let proposal_p1 = proposal_for("87c182d", "src/dual_control_docs.rs")
        .map_err(|error| format!("fixture proposal P1 invalid: {error}"))?;
    promote_once(&proposal_p1, &record_before, &registry)
        .map_err(|error| format!("P1 promotion rejected: {error}"))?;
    // The action's parameters change: different baseline, different
    // rollback target, different changed surface. The second approval is
    // equivalent (same operator, same bound candidate digest) — and it
    // promotes P2 with no re-approval of the changed parameters, because
    // the gate never compares the bound digest to the proposal.
    let proposal_p2 = proposal_for("de4db33f", "src/other_surface.rs")
        .map_err(|error| format!("fixture proposal P2 invalid: {error}"))?;
    promote_once(&proposal_p2, &record_after, &registry)
        .map_err(|error| format!("P2 promotion rejected: {error}"))?;
    evidence.push(
        "parameters changed (baseline 87c182d -> de4db33f, different changed \
         surface); an equivalent approval promotes the changed proposal — \
         the gate never compared approval.candidate_digest() to the \
         proposal, so no second approval was required"
            .to_string(),
    );
    Err(
        "dual control unmet: parameter change after approval requires no \
         second approval — the candidate-digest binding in the record is \
         signed but never verified against the promoted proposal"
            .to_string(),
    )
}

/// Attempt the task: run all four scenarios and aggregate into a
/// [`TaskOutcome`].
///
/// The task passes only if every dual-control requirement is met. The
/// gate takes exactly one approval per promotion with no quorum
/// parameter, so three of the four requirements are unmet: the verdict
/// is fail at the dual-control seam.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    let scenarios = [
        scenario_single_genuine_approval_promotes(),
        scenario_two_distinct_operators_no_quorum(),
        scenario_same_operator_twice_not_distinguished(),
        scenario_params_change_needs_no_second_approval(),
    ];
    let mut unmet: Vec<&str> = Vec::new();
    let mut outcome_evidence: Vec<String> = Vec::new();
    let mut report = format!("# {ID}: {NAME}\n\n");
    for scenario in &scenarios {
        let status = if scenario.requirement_met {
            "MET"
        } else {
            unmet.push(scenario.name);
            "NOT MET"
        };
        let kind = if scenario.validation {
            "validation"
        } else {
            "adversarial"
        };
        report.push_str(&format!("## {} [{status}] ({kind})\n", scenario.name));
        outcome_evidence.push(format!(
            "scenario {} ({kind}): requirement {status}",
            scenario.name
        ));
        for line in &scenario.evidence {
            report.push_str(&format!("- {line}\n"));
            outcome_evidence.push(format!("  {line}"));
        }
    }
    let report_dir = ctx.work_dir.join("task-58");
    let write_result = std::fs::create_dir_all(&report_dir)
        .and_then(|()| std::fs::write(report_dir.join("report.md"), &report))
        .map_err(|error| error.to_string());
    if let Err(reason) = write_result {
        return TaskOutcome::Fail {
            where_: "report-write".to_string(),
            how: "could not write the task report under the work dir".to_string(),
            evidence: bound_evidence(vec![format!("io error: {reason}")]),
        };
    }
    if unmet.is_empty() {
        TaskOutcome::Pass {
            evidence: bound_evidence(outcome_evidence),
        }
    } else {
        TaskOutcome::Fail {
            where_: "dual-control-seam".to_string(),
            how: format!(
                "{} of {} dual-control requirements not met: {}. \
                 PromotionGate::promote takes exactly one HumanApproval; \
                 no quorum parameter, no cross-approval distinctness, and \
                 the record's candidate-digest binding is never verified \
                 against the proposal",
                unmet.len(),
                scenarios.len(),
                unmet.join(", ")
            ),
            evidence: bound_evidence(outcome_evidence),
        }
    }
}
