//! Integration tests for task-100 (artifact swap between pipeline stages).
//!
//! The seam is ABSENT: phlow-experiment's `PromotionGate::promote`
//! takes the proposal, the `HumanApproval`, and the `EvidenceBundle`
//! as three independent arguments and never binds them to each other —
//! the evidence bundle's artifact digests are checked for completeness
//! only, never compared to the proposal; the approval's candidate
//! digest is copied into the `PromotionRecord` but never compared to
//! the proposal (task-91's banked gap); and the record carries no
//! content hash of the approved artifact. There is no apply stage in
//! code at all (apply is human-driven merge by design): no `fn apply`
//! exists in `crates/phlow-experiment/src`, so no stage re-hashes the
//! artifact after approval.
//!
//! Four cases — 2 validation, 2 adversarial. Both adversarial cases
//! are expected to FAIL as cases (the swaps succeed / are
//! undetectable); the task-level driver then reports the honest `fail`
//! at `"swap_between_validate_and_approve"`.
//!
//! Product decision banked for Matt: whether the pipeline should gain
//! cross-stage artifact hash chaining.

use phlow_gauntlet::tasks::task_100;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-100: cannot build Ctx: {e}"))
}

/// Unwrap the expected `fail` at `"swap_between_validate_and_approve"`,
/// or panic with the details.
fn fail_at_swap(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-100 passed: a stage hash-chain was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: the same (benign) artifact through propose → validate → approve
/// succeeds — the unchained pipeline works when nothing is swapped.
#[test]
fn same_artifact_all_stages() {
    assert_eq!(task_100::ID, "task-100");
    assert_eq!(task_100::NAME, "artifact swap between pipeline stages");
    assert_eq!(task_100::KIND, TaskKind::Rust);
    assert_eq!(task_100::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_100::run_case("same_artifact_all_stages")
        .unwrap_or_else(|e| panic!("task-100 case failed to run: {e}"));
    assert!(
        report.passed,
        "same-artifact case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["promoted"], true);
}

/// V2: the emitted promotion record carries the approval's candidate
/// digest (copied, uncompared) and no hash computed over the
/// proposal's content — the structural reason swaps are undetectable.
#[test]
fn record_carries_no_content_hash() {
    let report = task_100::run_case("record_carries_no_content_hash")
        .unwrap_or_else(|e| panic!("task-100 case failed to run: {e}"));
    assert!(
        report.passed,
        "record-fields case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["record_has_content_hash"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("copied, uncompared"),
        "evidence must show the digest is copied from the approval: \n{joined}"
    );
}

// --- adversarial ---

/// A1: the validated-twin swap — evidence built for the BENIGN twin,
/// proposal promoted is the MALICIOUS twin — succeeds. The case FAILS
/// (demonstrating the gap); the task-level driver then fails at
/// `"swap_between_validate_and_approve"`, and the `how` banks the
/// product decision for Matt.
#[test]
fn swap_between_validate_and_approve() {
    let report = task_100::run_case("swap_between_validate_and_approve")
        .unwrap_or_else(|e| panic!("task-100 case failed to run: {e}"));
    assert!(
        !report.passed,
        "the validate→approve swap case must FAIL while no stage binding exists: {}",
        report.evidence.join("\n")
    );
    let failure = report.failures.join("; ");
    assert!(
        failure.contains("swap_between_validate_and_approve"),
        "the case failure must name the gap: {failure}"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("SWAPPED AND PROMOTED"),
        "evidence must show the swap succeeded: \n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the product decision for Matt.
    let (where_, how, _) = fail_at_swap(task_100::run(&ctx()));
    assert_eq!(
        where_, "swap_between_validate_and_approve",
        "task-100 must fail at swap_between_validate_and_approve"
    );
    assert!(
        how.contains("Product decision banked"),
        "the 'how' must bank the product decision: {how}"
    );
}

/// A2: the approve→apply transition has no artifact verification —
/// apply is human-driven merge with no re-hash, the record carries no
/// content hash, and no apply-stage API exists. A post-approval swap
/// is undetectable in-band.
#[test]
fn swap_between_approve_and_apply() {
    let report = task_100::run_case("swap_between_approve_and_apply")
        .unwrap_or_else(|e| panic!("task-100 case failed to run: {e}"));
    assert!(
        !report.passed,
        "the approve→apply case must FAIL while no apply-stage verifier exists: {}",
        report.evidence.join("\n")
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no `fn apply`"),
        "evidence must show the absent apply stage: \n{joined}"
    );
}
