//! Integration tests for task-93 (improvement sandbox validation).
//!
//! The seam is ABSENT as designed: `phlow-experiment::Evaluator::prepare`
//! — documented as "Prepares the isolated workspace and snapshots" — is
//! implemented as `self.advance(EvalStage::Prepare)` with the source
//! comment "Skeleton: order only". `Lifecycle::Isolated` exists only as
//! state vocabulary ("Candidate workspace created in isolation"). No
//! network isolation exists anywhere in the workspace, no
//! write-containment exists, and no typed policy-violation reporting
//! exists.
//!
//! Four cases — 2 validation, 2 adversarial — each a bounded source
//! recon that fails closed (premise changed) if sandbox vocabulary ever
//! appears in a product crate. The driver reports the honest `fail` at
//! `"seam"`.
//!
//! Product decision banked for Matt: whether to build a genuine
//! network-disabled, write-contained validation sandbox with typed
//! policy-violation reporting.

use phlow_gauntlet::tasks::task_93;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-93: cannot build Ctx: {e}"))
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
            "task-93 passed: a validation sandbox was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: `Evaluator::prepare` is the documented skeleton — its body is
/// `self.advance(EvalStage::Prepare)`, order enforcement only. No
/// workspace is created, no snapshot taken.
#[test]
fn prepare_is_skeleton() {
    assert_eq!(task_93::ID, "task-93");
    assert_eq!(task_93::NAME, "improvement sandbox validation");
    assert_eq!(task_93::KIND, TaskKind::Rust);
    assert_eq!(task_93::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_93::run_case("prepare_is_skeleton")
        .unwrap_or_else(|e| panic!("task-93 case failed to run: {e}"));
    assert!(
        report.passed,
        "prepare-skeleton case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["workspace_created"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("Skeleton: order only"),
        "evidence must quote the skeleton marker: \n{joined}"
    );
}

/// V2: no network-isolation vocabulary exists in any product crate —
/// no `unshare`, no network namespace, no firewall rule, no
/// offline-sandbox flag for candidate evaluation. The task-level
/// driver then fails at the seam, and the `how` banks the product
/// decision for Matt.
#[test]
fn no_network_isolation() {
    let report = task_93::run_case("no_network_isolation")
        .unwrap_or_else(|e| panic!("task-93 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-network-isolation case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["hits"], 0);
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the product decision for Matt.
    let (where_, how, _) = fail_at_seam(task_93::run(&ctx()));
    assert_eq!(where_, "seam", "task-93 must fail at the seam");
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

/// A1: no write-containment vocabulary exists in any product crate —
/// no chroot, no read-only bind mount, no write-allowlist for the
/// candidate workspace. An escaping proposal's writes are not denied
/// by any sandbox layer.
#[test]
fn no_write_containment() {
    let report = task_93::run_case("no_write_containment")
        .unwrap_or_else(|e| panic!("task-93 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-write-containment case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("policy violation"),
        "evidence must name the missing policy-violation distinction: \n{joined}"
    );
}

/// A2: the isolation is advertised but unimplemented.
/// `Lifecycle::Isolated` ("Candidate workspace created in isolation")
/// exists as a state, and `EvalStage::Prepare` is named "Prepare the
/// isolated workspace and immutable snapshots" — but the transition
/// into `Isolated` performs no isolation. Names and docs promise a
/// sandbox; the code does not build one.
#[test]
fn isolation_advertised_but_unimplemented() {
    let report = task_93::run_case("isolation_advertised_but_unimplemented")
        .unwrap_or_else(|e| panic!("task-93 case failed to run: {e}"));
    assert!(
        report.passed,
        "advertised-isolation case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["sandbox_implemented"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("documented skeleton"),
        "evidence must name the honest labeling: \n{joined}"
    );
}
