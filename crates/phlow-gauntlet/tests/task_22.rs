//! Integration tests for task-22 (DAG diamond dependencies).
//!
//! The seam is ABSENT: phlow ships no DAG executor — the experiment
//! Scheduler is an admission ledger, `admit` stores `dependency_ids`
//! metadata without enforcing it. The four cases pin that honest finding
//! against phlow's real scheduler sources (compiled in via `#[path]`):
//! 2 validation, 2 adversarial.
//!
//! Each test compiles its own scenario binary into a unique workdir
//! (pid + process-local counter): parallel tests never share a binary.
//! `task_22::run` itself reports `fail` with `where = "seam"`; that
//! assertion is folded into the last test to keep the 2V/2A count.

use phlow_gauntlet::tasks::task_22;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-local sequence so concurrent tests never share a workdir.
static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Compile the scenario binary into a unique scratch dir.
fn scenario_binary(case: &str) -> PathBuf {
    let seq = WORKDIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let work_dir = std::env::temp_dir().join(format!(
        "gauntlet-task-22-{case}-{}-{seq}",
        std::process::id()
    ));
    std::fs::create_dir_all(&work_dir)
        .unwrap_or_else(|e| panic!("task-22: cannot create workdir: {e}"));
    task_22::ensure_scenario_binary(&work_dir)
        .unwrap_or_else(|e| panic!("task-22: scenario compile failed for '{case}': {e}"))
}

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(binary: &Path, case: &str) -> task_22::CaseReport {
    let report = task_22::run_case(binary, case)
        .unwrap_or_else(|e| panic!("task-22: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-22 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let seq = WORKDIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-22-run-{}-{seq}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-22 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-22 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; the diamond admits A→{B,C}→D with
/// `dependency_ids` stored faithfully on every node (the ledger records
/// the edges even though nothing enforces them).
#[test]
fn diamond_admit_stores_edges_faithfully() {
    assert_eq!(task_22::ID, "task-22");
    assert_eq!(task_22::NAME, "DAG diamond dependencies");
    assert_eq!(task_22::KIND, TaskKind::Rust);
    assert_eq!(
        task_22::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let binary = scenario_binary("diamond_admit");
    let report = run_case(&binary, "diamond_admit");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("edge metadata intact"),
        "evidence must show the edge metadata was stored:\n{joined}"
    );
}

/// V: admitting D with unadmitted (ghost) dependencies still succeeds —
/// the behavioral proof that `Scheduler::admit` never reads
/// `dependency_ids`. This case passing IS the absence finding: the edges
/// are inert metadata.
#[test]
fn dependencies_are_not_enforced_by_admit() {
    let binary = scenario_binary("dependencies_not_enforced");
    let report = run_case(&binary, "dependencies_not_enforced");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("edges are not enforced by admit"),
        "evidence must state the non-enforcement finding:\n{joined}"
    );
    assert!(
        joined.contains("behavioral proof"),
        "evidence must tie the case to the recon claim:\n{joined}"
    );
}

// --- adversarial ---

/// A: at-most-once publication — D's "trigger" firing twice commits only
/// the first digest; the second publish is refused with `DuplicateResult`
/// and the committed digest never changes.
#[test]
fn duplicate_publish_is_rejected_digest_immutable() {
    let binary = scenario_binary("duplicate_admit_rejected");
    let report = run_case(&binary, "duplicate_admit_rejected");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("DuplicateResult"),
        "evidence must show the second publish was refused:\n{joined}"
    );
    assert!(
        joined.contains("immutable"),
        "evidence must show the committed digest is immutable:\n{joined}"
    );
}

/// A: B and C "racing" to read A's output both see the same committed
/// digest — no torn read. Folded in: the task-level `run` reports the
/// evidenced seam absence (`where = "seam"`), keeping the 2V/2A count.
#[test]
fn shared_output_read_is_stable_and_task_reports_seam() {
    let binary = scenario_binary("shared_output_read");
    let report = run_case(&binary, "shared_output_read");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("same") || joined.contains("uniformly"),
        "evidence must show both readers saw the same digest:\n{joined}"
    );
    match task_22::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-22 must fail at the absent seam");
            assert!(
                how.contains("no DAG executor"),
                "the 'how' must name the absent executor: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-22 passed: the DAG executor seam was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
