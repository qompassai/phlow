//! Integration tests for task-29 (lease fencing).
//!
//! The seam is ABSENT: phlow has no distributed lock, lease, or fencing
//! primitive — token scans of `phlow-experiment`, `phlow-runtime`, and
//! `phlow-agent` find no lease/fencing machinery, and behavioral probes
//! against the real experiment `Scheduler` show the closest mechanism
//! (generation-checked publication) is at-most-once publication, not
//! mutual exclusion. The four cases pin that honest finding: 2
//! validation, 2 adversarial.
//!
//! `task_29::run` itself reports `fail` with `where = "seam"`; that
//! assertion is folded into the last test to keep the 2V/2A count.

use phlow_gauntlet::tasks::task_29;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_29::CaseReport {
    let report = task_29::run_case(case)
        .unwrap_or_else(|e| panic!("task-29: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-29 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-29-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-29 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-29 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; the token scan over the
/// experiment crate finds no lease/fencing machinery.
#[test]
fn no_lease_primitive_in_experiment() {
    assert_eq!(task_29::ID, "task-29");
    assert_eq!(task_29::NAME, "lease fencing");
    assert_eq!(task_29::KIND, TaskKind::Rust);
    assert_eq!(
        task_29::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("no_lease_primitive_in_experiment");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["lease_tokens"], 0,
        "no lease tokens may appear in phlow-experiment:\\n{joined}"
    );
    assert!(
        joined.contains("no lease/fencing tokens"),
        "evidence must state the scan result:\\n{joined}"
    );
}

/// V: the token scans over the runtime and agent crates find no
/// lease/fencing machinery either.
#[test]
fn no_lease_primitive_in_runtime_or_agent() {
    let report = run_case("no_lease_primitive_in_runtime_or_agent");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["lease_tokens"], 0,
        "no lease tokens may appear in runtime or agent crates:\\n{joined}"
    );
    assert!(
        joined.contains("phlow-runtime"),
        "evidence must name the runtime crate:\\n{joined}"
    );
    assert!(
        joined.contains("phlow-agent"),
        "evidence must name the agent crate:\\n{joined}"
    );
}

// --- adversarial ---

/// A: the closest existing mechanism — generation-checked publication —
/// is not fencing: a wrong-generation publish is rejected
/// (`StaleGeneration`), a second publish for the same generation is
/// refused (`DuplicateResult`), but no lease is acquired, no holder
/// identity exists, and nothing expires. At-most-once, not mutual
/// exclusion.
#[test]
fn generation_check_is_not_fencing() {
    let report = run_case("generation_check_is_not_fencing");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["stale_rejected"], true,
        "wrong-generation publish must be rejected:\\n{joined}"
    );
    assert_eq!(
        report.metrics["double_publish_refused"], true,
        "second publish must be refused:\\n{joined}"
    );
    assert!(
        joined.contains("StaleGeneration"),
        "evidence must name the stale-generation rejection:\\n{joined}"
    );
    assert!(
        joined.contains("DuplicateResult"),
        "evidence must name the duplicate-result refusal:\\n{joined}"
    );
    assert!(
        joined.contains("not fencing"),
        "evidence must state this is not fencing:\\n{joined}"
    );
}

/// A: two independent "workers" (separate scheduler instances) admit the
/// same key — both succeed, because there is no shared lease store to
/// contend on. The design's contention scenario is unrepresentable.
/// Folded in: the task-level `run` reports the evidenced seam absence
/// (`where = "seam"`), keeping the 2V/2A count.
#[test]
fn two_workers_cannot_contend_and_task_reports_seam() {
    let report = run_case("two_workers_cannot_contend");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["admissions_ok"], 2,
        "both workers must admit with no contention:\\n{joined}"
    );
    assert_eq!(
        report.metrics["contention_possible"], false,
        "contention must be impossible:\\n{joined}"
    );
    match task_29::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-29 must fail at the absent seam");
            assert!(
                how.contains("no distributed lock"),
                "the 'how' must name the absent primitive: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => {
            panic!("task-29 passed: the lease seam was invented, not found\nevidence: {evidence:?}")
        }
    }
}
