//! Integration tests for task-91 (propose→approve→apply).
//!
//! The seam is REAL but incomplete: phlow-experiment's
//! `ImprovementProposal` → `HumanApproval` → `PromotionGate::promote` →
//! `PromotionRecord` pipeline genuinely verifies dual signatures and
//! rejects replays, and the approval is required by value (no
//! approval-less path). The critical gap: the gate copies
//! `approval.candidate_digest()` into the promotion record but NEVER
//! compares it to the proposal — `ImprovementProposal` has no
//! candidate-digest field. A genuine approval for candidate X promotes
//! unrelated proposal Y. Apply is human-driven merge by design; no
//! autonomous tree mutation exists.
//!
//! Four cases — 2 validation, 2 adversarial. The A1 case is expected
//! to FAIL as a case (it demonstrates the gap); the task-level driver
//! then reports the honest `fail` at `"approval_candidate_mismatch"`.
//!
//! Product decision banked for Matt: whether to add proposal-content
//! hashing and approval↔proposal digest verification.

use phlow_gauntlet::tasks::task_91;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-91: cannot build Ctx: {e}"))
}

/// Unwrap the expected `fail` at `"approval_candidate_mismatch"`, or
/// panic with the details.
fn fail_at_mismatch(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-91 passed: the approval↔proposal digest check was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: a genuine dual-signed approval from an enrolled operator
/// promotes a well-formed proposal — the pipeline works end to end,
/// and the record links proposal, approval, and candidate digest.
/// The apply step is human-driven merge by design, so no applied-tree
/// hash exists in-band.
#[test]
fn genuine_dual_signed_approval_promotes() {
    assert_eq!(task_91::ID, "task-91");
    assert_eq!(task_91::NAME, "propose→approve→apply pipeline");
    assert_eq!(task_91::KIND, TaskKind::Rust);
    assert_eq!(task_91::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_91::run_case("genuine_approval_promotes")
        .unwrap_or_else(|e| panic!("task-91 case failed to run: {e}"));
    assert!(
        report.passed,
        "genuine-approval case must hold: {}",
        report.failures.join("; ")
    );
    assert!(
        report.metrics["evidence_checks"].is_number(),
        "metrics must carry the evidence-check count: {}",
        report.metrics
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("human-driven merge by design"),
        "evidence must name the human-driven apply: \n{joined}"
    );
}

/// V2: there is no promotion path without a `HumanApproval` token.
/// The gate takes the approval by value — the type system enforces
/// it, stronger than a runtime `approval_missing` check. Empty and
/// garbage records are rejected at parse.
#[test]
fn no_approval_no_promotion_path() {
    let report = task_91::run_case("no_approval_no_promotion_path")
        .unwrap_or_else(|e| panic!("task-91 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-approval case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["bypass_paths"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("by value"),
        "evidence must name the by-value enforcement: \n{joined}"
    );
}

// --- adversarial ---

/// A1: a GENUINE approval (valid dual signatures, enrolled operator)
/// covering candidate digest X promotes unrelated proposal Y — the
/// gate never compares the approval's digest against the proposal.
/// The case FAILS (demonstrating the gap); the task-level driver then
/// fails at `"approval_candidate_mismatch"`, and the `how` banks the
/// product decision for Matt.
#[test]
fn approval_candidate_mismatch() {
    let report = task_91::run_case("approval_candidate_mismatch")
        .unwrap_or_else(|e| panic!("task-91 case failed to run: {e}"));
    assert!(
        !report.passed,
        "the mismatch case must FAIL while the digest check is absent: {}",
        report.evidence.join("\n")
    );
    let failure = report.failures.join("; ");
    assert!(
        failure.contains("approval_mismatch"),
        "the case failure must name the missing check: {failure}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the product decision for Matt.
    let (where_, how, _) = fail_at_mismatch(task_91::run(&ctx()));
    assert_eq!(
        where_, "approval_candidate_mismatch",
        "task-91 must fail at approval_candidate_mismatch"
    );
    assert!(
        how.contains("no approval_mismatch check exists"),
        "the 'how' must name the missing check: {how}"
    );
    assert!(
        how.contains("Product decision banked"),
        "the 'how' must bank the product decision: {how}"
    );
}

/// A2: the same approval id used twice — the second promotion is
/// rejected with `ApprovalReplayed`. By-value consumption plus the
/// replay store close both the token-reuse and the re-parse holes.
#[test]
fn replay_rejected() {
    let report = task_91::run_case("replay_rejected")
        .unwrap_or_else(|e| panic!("task-91 case failed to run: {e}"));
    assert!(
        report.passed,
        "replay case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["replay_rejected"], true);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("ApprovalReplayed"),
        "evidence must name the replay rejection: \n{joined}"
    );
}
