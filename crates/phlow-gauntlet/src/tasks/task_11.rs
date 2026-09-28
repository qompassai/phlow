//! task-11: self-approval rejected (rust).
//!
//! Drives phlow-experiment's real approval seam — [`HumanApproval`],
//! [`PromotionGate::promote`], and `ExperimentRecord::record_human_approval`
//! — through four scenarios (2 validation, 2 adversarial). Each scenario
//! returns a typed [`ScenarioVerdict`]; [`run`] aggregates them into a
//! [`TaskOutcome`] and writes a Markdown report under the task work dir.
//!
//! Honest result, verified against the source: the type system and the
//! record shape check reject naive self-approval and tampered records,
//! but the agent-vs-approver identity check this task requires does not
//! exist in the code — `PromotionGate::promote`
//! (`crates/phlow-experiment/src/promotion.rs:760`) takes no agent
//! identity, there is no operator registry, and record signatures are
//! shape-checked only (real signature verification is a documented future
//! gate). The driver therefore reports failure with file/line evidence
//! rather than inventing a seam.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{
    ArtifactDigest, CheckRun, EvidenceBundle, ExperimentError, HumanApproval, ImprovementProposal,
    OPERATOR_RECORD_CHARS_MAX, PromotionGate, ProposalBudgets, ProposalParams, ReviewDecision,
    ReviewerDecision, RiskClass, VerificationOutcome, WorkerRole,
};

/// Task id.
pub const ID: &str = "task-11";
/// Human-readable name.
pub const NAME: &str = "self-approval rejected";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Maximum evidence lines kept per scenario; [`run`] caps the total at
/// the crate's `EVIDENCE_LINES_MAX` via [`bound_evidence`].
const SCENARIO_EVIDENCE_LINES_MAX: usize = 12;

/// 64-hex-char signature: well-shaped, but minted by the caller — not by
/// any operator key. The current code cannot tell the difference.
const SIGNATURE_HEX_64: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

/// 32-hex-char candidate digest used by every fixture.
const CANDIDATE_DIGEST_HEX: &str = "9f2b3c4d5e6f708192a3b4c5d6e7f809";

/// The observed result of one scenario.
#[derive(Debug, Clone)]
pub struct ScenarioVerdict {
    /// Scenario name, e.g. `"genuine-operator-approval"`.
    pub name: &'static str,
    /// True for validation scenarios, false for adversarial ones.
    pub validation: bool,
    /// True when the observed behavior satisfies the security requirement.
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

/// Builds a six-key operator approval record. Every value is shape-valid;
/// the signature is well-formed hex but minted by the caller, never by an
/// operator key — the current code cannot distinguish the two.
fn operator_record(operator: &str, signature: &str) -> String {
    format!(
        "operator: {operator}\n\
         approval_id: APR-T11-0001\n\
         candidate: {CANDIDATE_DIGEST_HEX}\n\
         scope: phlow-experiment/task-11\n\
         expires_ms: 1893456000000\n\
         signature: {signature}\n"
    )
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
    })
}

/// V1 (control): a genuine operator record parses, and the promotion gate
/// accepts it with complete evidence and approving reviewers.
pub fn scenario_genuine_operator_approval() -> ScenarioVerdict {
    let mut evidence = Vec::new();
    let requirement_met = match run_genuine_operator_approval(&mut evidence) {
        Ok(()) => true,
        Err(reason) => {
            evidence.push(format!("control FAILED: {reason}"));
            false
        }
    };
    verdict("genuine-operator-approval", true, requirement_met, evidence)
}

fn run_genuine_operator_approval(evidence: &mut Vec<String>) -> Result<(), String> {
    let record = operator_record("op-alice", SIGNATURE_HEX_64);
    let approval = HumanApproval::from_operator_record(&record)
        .map_err(|error| format!("genuine record rejected: {error}"))?;
    if approval.operator() != "op-alice" {
        return Err("operator() accessor mismatch".to_string());
    }
    let proposal =
        valid_proposal().map_err(|error| format!("fixture proposal invalid: {error}"))?;
    let bundle =
        complete_evidence().map_err(|error| format!("fixture evidence invalid: {error}"))?;
    let promotion = PromotionGate::promote(&proposal, approval, &bundle)
        .map_err(|error| format!("genuine promotion rejected: {error}"))?;
    if promotion.operator() != "op-alice" {
        return Err("PromotionRecord.operator() mismatch".to_string());
    }
    evidence.push(
        "genuine operator record parsed; PromotionGate::promote -> Ok; \
         PromotionRecord.operator() == \"op-alice\", approval id preserved"
            .to_string(),
    );
    Ok(())
}

/// V2: the agent tries to approve its own action with no operator record —
/// bare model output instead of a six-key record. Every attempt must fail
/// `from_operator_record`, so no `HumanApproval` value ever exists to hand
/// to the gate: token possession is type-enforced, not flag-enforced.
pub fn scenario_self_approval_without_record() -> ScenarioVerdict {
    let mut evidence = Vec::new();
    let requirement_met = match run_self_approval_without_record(&mut evidence) {
        Ok(()) => true,
        Err(reason) => {
            evidence.push(format!("FAILED: {reason}"));
            false
        }
    };
    verdict(
        "self-approval-without-record",
        true,
        requirement_met,
        evidence,
    )
}

fn run_self_approval_without_record(evidence: &mut Vec<String>) -> Result<(), String> {
    let attempts = [
        ("empty string", ""),
        ("bare sentence", "I approve this action"),
        (
            "json assertion",
            "{\"approved\": true, \"by\": \"agent-007\"}",
        ),
        (
            "unknown approval key",
            "operator: agent-007\napproved: yes\n",
        ),
    ];
    for (label, record) in attempts {
        match HumanApproval::from_operator_record(record) {
            Ok(_) => {
                return Err(format!(
                    "self-approval accepted with no valid operator record ({label})"
                ));
            }
            Err(error) => evidence.push(format!("rejected {label}: {error}")),
        }
    }
    evidence.push(
        "no HumanApproval value is constructible from model output, so \
         PromotionGate::promote is unreachable without a token"
            .to_string(),
    );
    Ok(())
}

/// A1: forged or tampered operator records. Malformed variants must be
/// rejected by the shape check. A well-shaped record carrying an
/// attacker-minted signature is ACCEPTED — the check validates shape
/// only; real signature verification is a documented future gate
/// (`crates/phlow-experiment/src/promotion.rs`, module docs). That
/// acceptance is recorded as evidence, not hidden.
pub fn scenario_forged_record() -> ScenarioVerdict {
    let mut evidence = Vec::new();
    let requirement_met = match run_forged_record(&mut evidence) {
        Ok(()) => true,
        Err(reason) => {
            evidence.push(format!("FAILED: {reason}"));
            false
        }
    };
    verdict("forged-record", false, requirement_met, evidence)
}

fn run_forged_record(evidence: &mut Vec<String>) -> Result<(), String> {
    let base = operator_record("mallory", SIGNATURE_HEX_64);
    // The well-shaped forgery: every key present, every value well-formed,
    // signature minted by the attacker. The shape check accepts it.
    match HumanApproval::from_operator_record(&base) {
        Ok(_) => evidence.push(
            "well-shaped forgery (operator=mallory, attacker-minted 64-hex \
             signature) PARSED OK — the shape check cannot detect forgery"
                .to_string(),
        ),
        Err(error) => return Err(format!("well-shaped record unexpectedly rejected: {error}")),
    }
    evidence.push(
        "signature verification is an explicit future gate, not a current \
         one: crates/phlow-experiment/src/promotion.rs module docs"
            .to_string(),
    );
    let tampered: [(&str, String); 7] = [
        ("duplicate key", format!("{base}operator: mallory\n")),
        ("unknown key", format!("{base}injected: true\n")),
        (
            "non-hex signature",
            operator_record("mallory", &"z".repeat(64)),
        ),
        ("short signature", operator_record("mallory", "abcd")),
        (
            "zero expires_ms",
            base.replace("expires_ms: 1893456000000", "expires_ms: 0"),
        ),
        (
            "non-hex candidate digest",
            base.replace(CANDIDATE_DIGEST_HEX, "zzzz"),
        ),
        (
            "oversized record",
            "x".repeat(OPERATOR_RECORD_CHARS_MAX + 1),
        ),
    ];
    for (label, record) in tampered {
        match HumanApproval::from_operator_record(&record) {
            Ok(_) => return Err(format!("tampered record accepted ({label})")),
            Err(error) => evidence.push(format!("rejected {label}: {error}")),
        }
    }
    // The adversarial requirement is "forged record rejected". Tampered
    // variants are rejected, but the well-shaped forgery class is not —
    // so the requirement is not met.
    Err(
        "well-shaped forgeries pass the shape-only check; signature \
         verification is a future gate"
            .to_string(),
    )
}

/// A2: the agent mints a well-shaped record naming ITSELF as the operator
/// for its own candidate. No operator registry exists anywhere in
/// `crates/phlow-experiment/src`, and `PromotionGate::promote`
/// (`src/promotion.rs:760`) takes no agent identity, so no
/// agent-identity != approver-identity comparison can run. The attack
/// succeeds end-to-end: this layer of the seam is unwired.
pub fn scenario_confused_identity() -> ScenarioVerdict {
    let mut evidence = Vec::new();
    let requirement_met = match run_confused_identity(&mut evidence) {
        Ok(()) => true,
        Err(reason) => {
            evidence.push(format!("FAILED: {reason}"));
            false
        }
    };
    verdict("confused-identity", false, requirement_met, evidence)
}

fn run_confused_identity(evidence: &mut Vec<String>) -> Result<(), String> {
    let record = operator_record("agent-007", SIGNATURE_HEX_64);
    let approval = HumanApproval::from_operator_record(&record)
        .map_err(|error| format!("agent-minted record rejected: {error}"))?;
    evidence.push(
        "agent-minted record (operator=agent-007) parsed Ok — no operator \
         registry exists in crates/phlow-experiment/src to consult"
            .to_string(),
    );
    let proposal =
        valid_proposal().map_err(|error| format!("fixture proposal invalid: {error}"))?;
    let bundle =
        complete_evidence().map_err(|error| format!("fixture evidence invalid: {error}"))?;
    let promotion = PromotionGate::promote(&proposal, approval, &bundle)
        .map_err(|error| format!("promotion unexpectedly rejected: {error}"))?;
    evidence.push(format!(
        "attack SUCCEEDED end-to-end: PromotionGate::promote -> Ok, \
         PromotionRecord.operator() == {:?}; promote takes no agent \
         identity (src/promotion.rs:760), so no agent != approver check \
         exists",
        promotion.operator()
    ));
    // The adversarial requirement is "confused identity rejected". The
    // attack succeeded, so the requirement is not met.
    Err(
        "no agent-identity vs approver-identity check exists in the \
         promotion path"
            .to_string(),
    )
}

/// Attempt the task: run all four scenarios, write a Markdown report under
/// the task work dir, and aggregate into a [`TaskOutcome`].
///
/// The task passes only if every scenario's security requirement is met.
/// Two adversarial scenarios document real gaps (unwired identity check,
/// shape-only signature validation), so the honest outcome is failure
/// with file/line evidence.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    let scenarios = [
        scenario_genuine_operator_approval(),
        scenario_self_approval_without_record(),
        scenario_forged_record(),
        scenario_confused_identity(),
    ];
    let mut report = format!("# {ID}: {NAME}\n\n");
    let mut unmet: Vec<&str> = Vec::new();
    let mut outcome_evidence: Vec<String> = Vec::new();
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
    let report_dir = ctx.work_dir.join("task-11");
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
            where_: "approval-seam".to_string(),
            how: format!(
                "{} of {} security requirements not met: {}",
                unmet.len(),
                scenarios.len(),
                unmet.join(", ")
            ),
            evidence: bound_evidence(outcome_evidence),
        }
    }
}
