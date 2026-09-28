//! Integration tests for task-98 (incremental composition attack).
//!
//! The seam is ABSENT: phlow-experiment evaluates one proposal per
//! `promote()` call and checks only that proposal's own changed
//! surface. No sequence API exists, no cumulative base→tip diff is
//! ever computed, and bounded exact-token scans over
//! `crates/phlow-experiment/src/**/*.rs` find zero hits for
//! cumulative/composition vocabulary. The only content policy is the
//! per-proposal protected-surface check — there is no compositional
//! policy and no cumulative walk (so the design's lldb-dap validation
//! of the walk is vacuous: there is nothing to walk).
//!
//! Four cases — 2 validation, 2 adversarial. Both adversarial cases
//! are expected to FAIL as cases (the composition is never evaluated);
//! the task-level driver then reports the honest `fail` at
//! `"three_step_composition_undefeated"`.
//!
//! Product decision banked for Matt: whether the pipeline should gain
//! cumulative base→tip diff evaluation with a compositional policy.

use phlow_gauntlet::tasks::task_98;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-98: cannot build Ctx: {e}"))
}

/// Unwrap the expected `fail` at `"three_step_composition_undefeated"`,
/// or panic with the details.
fn fail_at_composition(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-98 passed: a cumulative evaluation was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: a single benign proposal promotes — the step-local pipeline
/// works end to end.
#[test]
fn single_benign_promotes() {
    assert_eq!(task_98::ID, "task-98");
    assert_eq!(task_98::NAME, "incremental composition attack");
    assert_eq!(task_98::KIND, TaskKind::Rust);
    assert_eq!(task_98::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_98::run_case("single_benign_promotes")
        .unwrap_or_else(|e| panic!("task-98 case failed to run: {e}"));
    assert!(
        report.passed,
        "single-benign case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["promoted"], true);
}

/// V2: each proposal of the attack sequence passes the step-local
/// surface check individually — the per-proposal review the design
/// assumes is real.
#[test]
fn step_local_review_exists() {
    let report = task_98::run_case("step_local_review_exists")
        .unwrap_or_else(|e| panic!("task-98 case failed to run: {e}"));
    assert!(
        report.passed,
        "step-local case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["steps_passing_step_local"], 3);
}

// --- adversarial ---

/// A1: the three-proposal sequence (logging helper → widened call
/// sites → network sink) promotes step by step with genuine approvals
/// — nothing evaluates the cumulative composition. The case FAILS
/// (demonstrating the gap); the task-level driver then fails at
/// `"three_step_composition_undefeated"`, and the `how` banks the
/// product decision for Matt.
#[test]
fn three_step_composition_undefeated() {
    let report = task_98::run_case("three_step_composition_undefeated")
        .unwrap_or_else(|e| panic!("task-98 case failed to run: {e}"));
    assert!(
        !report.passed,
        "the composition case must FAIL while no cumulative evaluation exists: {}",
        report.evidence.join("\n")
    );
    let failure = report.failures.join("; ");
    assert!(
        failure.contains("three_step_composition_undefeated"),
        "the case failure must name the gap: {failure}"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("step TASK98-103 promoted"),
        "evidence must show all three steps promoted: \n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the product decision for Matt.
    let (where_, how, _) = fail_at_composition(task_98::run(&ctx()));
    assert_eq!(
        where_, "three_step_composition_undefeated",
        "task-98 must fail at three_step_composition_undefeated"
    );
    assert!(
        how.contains("Product decision banked"),
        "the 'how' must bank the product decision: {how}"
    );
}

/// A2: twelve tiny proposals each pass the step-local check, and the
/// recon confirms no cumulative machinery exists — splitting
/// trivially evades per-proposal scrutiny because there is nothing
/// cumulative to evade.
#[test]
fn split_proposals_evade_scrutiny() {
    let report = task_98::run_case("split_proposals_evade_scrutiny")
        .unwrap_or_else(|e| panic!("task-98 case failed to run: {e}"));
    assert!(
        !report.passed,
        "the splitting case must FAIL while no cumulative check exists: {}",
        report.evidence.join("\n")
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("12 tiny proposals"),
        "evidence must show the split sequence passed step-local: \n{joined}"
    );
    assert!(
        joined.contains("0 hit(s)"),
        "evidence must show the zero-hit recon: \n{joined}"
    );
}
