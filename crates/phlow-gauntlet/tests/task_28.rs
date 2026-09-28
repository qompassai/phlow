//! Integration tests for task-28 (idempotency keys).
//!
//! The seam EXISTS: phlow's real experiment `Scheduler` keys admission
//! by caller-supplied `NodeId` — the idempotency key — and refuses a
//! second admission with `ExperimentError::DuplicateNode`, so the side
//! effect (admission) runs exactly once per key. The four cases drive the
//! real `Scheduler` (no mocks): 2 validation, 2 adversarial.
//!
//! Documented semantic deltas (not hidden): the second submission gets
//! `DuplicateNode` (an error), not the first result handle — the client
//! must catch-and-refetch; and there is no key TTL, so keys never become
//! "new" again (the admitted key set grows without bound).
//!
//! `task_28::run` itself reports `pass`; that assertion is folded into
//! the last test to keep the 2V/2A count.

use phlow_gauntlet::tasks::task_28;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_28::CaseReport {
    let report = task_28::run_case(case)
        .unwrap_or_else(|e| panic!("task-28: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-28 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-28-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-28 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-28 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; the same key submitted twice
/// admits once — the second submission is rejected with
/// `DuplicateNode` naming the key.
#[test]
fn same_key_twice_admits_once() {
    assert_eq!(task_28::ID, "task-28");
    assert_eq!(task_28::NAME, "idempotency keys");
    assert_eq!(task_28::KIND, TaskKind::Rust);
    assert_eq!(
        task_28::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("same_key_twice_admits_once");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["admissions_ok"], 1,
        "the key must admit exactly once:\\n{joined}"
    );
    assert_eq!(
        report.metrics["duplicates_rejected"], 1,
        "the second submission must be rejected:\\n{joined}"
    );
    assert!(
        joined.contains("DuplicateNode"),
        "evidence must name the rejection error:\\n{joined}"
    );
}

/// V: different keys admit twice — the dedup is per-key, not global.
#[test]
fn different_keys_admit_twice() {
    let report = run_case("different_keys_admit_twice");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["admissions_ok"], 2,
        "two distinct keys must both admit:\\n{joined}"
    );
    assert!(
        joined.contains("per-key"),
        "evidence must state dedup is per-key:\\n{joined}"
    );
}

// --- adversarial ---

/// A: the same key re-submitted with a DIFFERENT payload is rejected as
/// a conflict — never silently executed, never overwriting the stored
/// node. The conflict error names the key.
#[test]
fn conflicting_payload_is_rejected_not_executed() {
    let report = run_case("same_key_different_payload_rejected");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["conflicts_rejected"], 1,
        "the conflicting re-submission must be rejected:\\n{joined}"
    );
    assert!(
        joined.contains("not silently executed"),
        "evidence must state the conflict was not executed:\\n{joined}"
    );
    assert!(
        joined.contains("payload-1"),
        "evidence must show the stored payload is unchanged:\\n{joined}"
    );
}

/// A: key expiry — there is none, and that is documented: an old key
/// re-submitted is rejected forever. Folded in: the task-level `run`
/// reports `pass` (the seam exists), keeping the 2V/2A count.
#[test]
fn old_keys_never_expire_and_task_passes() {
    let report = run_case("no_key_expiry_documented");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["expiry_mechanism"], "none",
        "there must be no TTL mechanism:\\n{joined}"
    );
    assert!(
        joined.contains("never expire"),
        "evidence must document permanent rejection:\\n{joined}"
    );
    assert!(
        joined.contains("grows without bound"),
        "evidence must document the unbounded-key-set cost:\\n{joined}"
    );
    match task_28::run(&test_ctx()) {
        TaskOutcome::Pass { evidence } => {
            let joined = evidence.join("\n");
            assert!(
                joined.contains("exactly once per key"),
                "task evidence must state the once-per-key property:\\n{joined}"
            );
        }
        TaskOutcome::Fail { where_, how, .. } => {
            panic!("task-28 failed at {where_}: the idempotency seam exists and should pass: {how}")
        }
    }
}
