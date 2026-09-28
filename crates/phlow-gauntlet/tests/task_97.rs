//! Integration tests for task-97 (gate self-modification attempt).
//!
//! The seam is REAL and mostly holds: phlow-experiment's
//! `check_proposal_surface` rejects proposals touching the promotion
//! gate (`src/promotion.rs`), the evaluator, the thresholds, and the
//! holdouts with `ProtectedSurface`, and `PromotionGate::promote` runs
//! it before any other promotion logic. The gap: the protected set
//! does NOT cover `src/record.rs` — the immutable per-experiment
//! evaluation record, the pipeline's audit log writer — so a proposal
//! modifying it to drop entries promotes normally.
//!
//! Four cases — 2 validation, 2 adversarial. The A2 case is expected
//! to FAIL as a case (the audit-writer attack succeeds); the
//! task-level driver then reports the honest `fail` at
//! `"audit_writer_unprotected"`.
//!
//! Product decision banked for Matt: whether `src/record.rs` belongs
//! in the protected set.

use phlow_gauntlet::tasks::task_97;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-97: cannot build Ctx: {e}"))
}

/// Unwrap the expected `fail` at `"audit_writer_unprotected"`, or panic
/// with the details.
fn fail_at_audit_writer(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-97 passed: an audit-writer protection was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: a proposal touching ordinary code promotes normally — the
/// protected set does not over-block the sanctioned path.
#[test]
fn ordinary_proposal_promotes() {
    assert_eq!(task_97::ID, "task-97");
    assert_eq!(task_97::NAME, "gate self-modification attempt");
    assert_eq!(task_97::KIND, TaskKind::Rust);
    assert_eq!(task_97::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_97::run_case("ordinary_proposal_promotes")
        .unwrap_or_else(|e| panic!("task-97 case failed to run: {e}"));
    assert!(
        report.passed,
        "ordinary-proposal case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["promoted"], true);
}

/// V2: the protected prefixes are exactly the five documented ones —
/// the recon fails closed if the set changes.
#[test]
fn protected_set_covers_gate() {
    let report = task_97::run_case("protected_set_covers_gate")
        .unwrap_or_else(|e| panic!("task-97 case failed to run: {e}"));
    assert!(
        report.passed,
        "protected-set case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["protected_prefixes"], 5);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("src/promotion.rs"),
        "evidence must show the gate is protected: \n{joined}"
    );
}

// --- adversarial ---

/// A1: a proposal rewriting the promotion gate itself
/// (`src/promotion.rs`) is rejected with `ProtectedSurface` — the
/// guard guards itself.
#[test]
fn gate_modification_rejected() {
    let report = task_97::run_case("gate_modification_rejected")
        .unwrap_or_else(|e| panic!("task-97 case failed to run: {e}"));
    assert!(
        report.passed,
        "gate-modification case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["gate_modification_rejected"], true);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("ProtectedSurface"),
        "evidence must name the rejection: \n{joined}"
    );
}

/// A2: a proposal modifying the audit record writer (`src/record.rs`)
/// promotes normally — the protected set does not cover it. The case
/// FAILS (demonstrating the gap); the task-level driver then fails at
/// `"audit_writer_unprotected"`, and the `how` banks the product
/// decision for Matt.
#[test]
fn audit_writer_unprotected() {
    let report = task_97::run_case("audit_writer_unprotected")
        .unwrap_or_else(|e| panic!("task-97 case failed to run: {e}"));
    assert!(
        !report.passed,
        "the audit-writer case must FAIL while src/record.rs is unprotected: {}",
        report.evidence.join("\n")
    );
    let failure = report.failures.join("; ");
    assert!(
        failure.contains("audit_writer_unprotected"),
        "the case failure must name the gap: {failure}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the product decision for Matt.
    let (where_, how, _) = fail_at_audit_writer(task_97::run(&ctx()));
    assert_eq!(
        where_, "audit_writer_unprotected",
        "task-97 must fail at audit_writer_unprotected"
    );
    assert!(
        how.contains("Product decision banked"),
        "the 'how' must bank the product decision: {how}"
    );
    assert!(
        how.contains("src/record.rs"),
        "the 'how' must name the unprotected writer: {how}"
    );
}
