//! Integration tests for task-94 (improvement rollback).
//!
//! The seam is ABSENT as designed: `ImprovementProposal.rollback_target`
//! ("The exact revision to restore on rollback") is a String field
//! copied into `PromotionRecord`; `Lifecycle::RolledBack` names the
//! terminal state; but promotion.rs contains no subprocess, git,
//! worktree, or checkout machinery that could act on it. The only
//! `revert` code paths in the workspace are explicit denials —
//! `phlow-self-improve::SkillStore::revert_skill` always fails with
//! `GitMutationDisabled` ("Automatic Git mutation is disabled; revert
//! manually").
//!
//! Four cases — 2 validation, 2 adversarial — each a bounded source
//! recon that fails closed (premise changed) if revert vocabulary ever
//! appears in a product crate. The driver reports the honest `fail` at
//! `"seam"`.
//!
//! Product decision banked for Matt: whether phlow-experiment should
//! gain a real gated rollback path.

use phlow_gauntlet::tasks::task_94;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-94: cannot build Ctx: {e}"))
}

/// Unwrap the expected `fail` at `"seam"`, or panic with the details.
fn fail_at_seam(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-94 passed: a rollback mechanism was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: no revert mechanism exists. The only `revert` code paths in the
/// workspace are explicit denials (`SkillStore::revert_skill` always
/// fails with `GitMutationDisabled`); no git-checkout/restore, no
/// worktree manipulation, no tree-restoring function exists.
#[test]
fn no_revert_mechanism() {
    assert_eq!(task_94::ID, "task-94");
    assert_eq!(task_94::NAME, "improvement rollback");
    assert_eq!(task_94::KIND, TaskKind::Rust);
    assert_eq!(task_94::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_94::run_case("no_revert_mechanism")
        .unwrap_or_else(|e| panic!("task-94 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-revert case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("GitMutationDisabled"),
        "evidence must name the explicit denial: \n{joined}"
    );
}

/// V2: `rollback_target` is data only — declared on
/// `ProposalParams`/`ImprovementProposal`, exposed via an accessor,
/// and copied into `PromotionRecord` — but nothing reads it to perform
/// a revert: no fs or git operation consumes it. The task-level driver
/// then fails at the seam, and the `how` banks the product decision
/// for Matt.
#[test]
fn rollback_target_is_data_only() {
    let report = task_94::run_case("rollback_target_is_data_only")
        .unwrap_or_else(|e| panic!("task-94 case failed to run: {e}"));
    assert!(
        report.passed,
        "rollback-target case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["revert_mechanisms"], 0);
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the product decision for Matt.
    let (where_, how, _) = fail_at_seam(task_94::run(&ctx()));
    assert_eq!(where_, "seam", "task-94 must fail at the seam");
    assert!(
        how.contains("seam absent"),
        "the 'how' must name the absent seam: {how}"
    );
    assert!(
        how.contains("Product decision banked"),
        "the 'how' must bank the product decision: {how}"
    );
}

// --- adversarial ---

/// A1: the design demands the revert be a gated, human-approved
/// action. With no revert path at all, that property is unverifiable —
/// it holds vacuously (nothing reverts, approved or not), which is
/// fail-closed but not the design's gated-revert. No approval-gated
/// revert entry point exists.
#[test]
fn revert_approval_unverifiable() {
    let report = task_94::run_case("revert_approval_unverifiable")
        .unwrap_or_else(|e| panic!("task-94 case failed to run: {e}"));
    assert!(
        report.passed,
        "approval-gate case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("fail-closed by absence"),
        "evidence must name the vacuous property: \n{joined}"
    );
}

/// A2: no unsafe-revert guard exists — no conflict detection, no
/// three-way merge, no 'revert refused as unsafe' path. A future
/// revert implementation would need all three; today there is nothing
/// to be unsafe.
#[test]
fn no_unsafe_revert_guard() {
    let report = task_94::run_case("no_unsafe_revert_guard")
        .unwrap_or_else(|e| panic!("task-94 case failed to run: {e}"));
    assert!(
        report.passed,
        "unsafe-revert case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no revert exists to guard"),
        "evidence must name the absent guard: \n{joined}"
    );
}
