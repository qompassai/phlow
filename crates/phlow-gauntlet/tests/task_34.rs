//! Integration tests for task-34 (adversarial concurrent merge).
//!
//! The seam is ABSENT: no merge function exists anywhere in the
//! workspace. Two scripted writers racing on one real `EvaluationRecord`
//! show disjoint fields do not clobber and concurrent appends both
//! survive — but a same-field concurrent write is silent
//! last-writer-wins: both writes acknowledged, the first lost, zero
//! conflicts surfaced. The four cases drive the real record (no mocks):
//! 2 validation, 2 adversarial.
//!
//! `task_34::run` itself reports `fail` at `"seam"` (an acknowledged
//! write was lost; the design's pass criteria are violated); that
//! assertion is folded into the last test to keep the 2V/2A count.

use phlow_gauntlet::tasks::task_34;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_34::CaseReport {
    let report = task_34::run_case(case)
        .unwrap_or_else(|e| panic!("task-34: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-34 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-34-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-34 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-34 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; the two writers touch disjoint
/// fields — both writes acknowledged, both survive, neither clobbers the
/// other.
#[test]
fn disjoint_fields_merge_cleanly() {
    assert_eq!(task_34::ID, "task-34");
    assert_eq!(task_34::NAME, "adversarial concurrent merge");
    assert_eq!(task_34::KIND, TaskKind::Rust);
    assert_eq!(
        task_34::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("disjoint_fields_merge_cleanly");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["writes_acknowledged"], 2,
        "both writes must be acknowledged:\n{joined}"
    );
    assert_eq!(
        report.metrics["writes_surviving"], 2,
        "both writes must survive:\n{joined}"
    );
}

/// V: both writers append to the same event list — both appends survive
/// in order. Appends never conflict, so this holds without any merge.
#[test]
fn concurrent_appends_both_survive() {
    let report = run_case("concurrent_appends_both_survive");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["appends_acknowledged"], 2,
        "both appends must be acknowledged:\n{joined}"
    );
    assert_eq!(
        report.metrics["appends_surviving"], 2,
        "both appends must survive:\n{joined}"
    );
}

// --- adversarial ---

/// A: both writers write the SAME field — both acknowledged, the second
/// silently overwrites the first. The design's adversarial case breaks:
/// an acknowledged write is lost with zero conflicts surfaced.
#[test]
fn same_field_write_loses_silently() {
    let report = run_case("same_field_write_is_silent_last_writer_wins");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["writes_acknowledged"], 2,
        "both conflicting writes must be acknowledged:\n{joined}"
    );
    assert_eq!(
        report.metrics["writes_surviving"], 1,
        "only one write survives the silent overwrite:\n{joined}"
    );
    assert_eq!(
        report.metrics["conflicts_surfaced"], 0,
        "no conflict may be surfaced — that is the failure:\n{joined}"
    );
    assert!(
        joined.contains("silently lost"),
        "evidence must document the silent loss:\n{joined}"
    );
}

/// A: after the silent loss, no conflict primitive exists anywhere —
/// no conflict marker, no version field, and no field-deletion API, so
/// the delete-vs-update scenario has no seam either. Folded in: the
/// task-level `run` reports `fail` at `"seam"`, keeping the 2V/2A count.
#[test]
fn no_conflict_ever_surfaced_and_task_fails_at_seam() {
    let report = run_case("no_conflict_is_ever_surfaced");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["conflict_markers"], 0,
        "no conflict marker may exist:\n{joined}"
    );
    assert_eq!(
        report.metrics["delete_apis"], 0,
        "no field-deletion API may exist:\n{joined}"
    );
    assert!(
        joined.contains("indistinguishable from a world where writer A never wrote"),
        "evidence must state the loss is unrecoverable:\n{joined}"
    );
    match task_34::run(&test_ctx()) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(where_, "seam", "task-34 must fail at the absent seam");
            let joined = evidence.join("\n");
            assert!(
                joined.contains("no merge function exists"),
                "evidence must name the absent merge:\n{joined}"
            );
            assert!(
                how.contains("no acknowledged write lost"),
                "the 'how' must name the lost acknowledged write: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-34 passed: a lossless merge was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
