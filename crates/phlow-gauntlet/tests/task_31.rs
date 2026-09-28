//! Integration tests for task-31 (optimistic concurrency).
//!
//! The seam is ABSENT: phlow has no versioned compare-and-swap write
//! path. `EvaluationRecord` mutations are plain last-writer-wins (no
//! per-record version counter), and `Scheduler::publish_result`'s
//! generation check pins the node's delegation depth — fixed at admission,
//! never bumped by writes — a stale-handle guard, not optimistic
//! concurrency. The four cases drive the real types (no mocks):
//! 2 validation, 2 adversarial.
//!
//! `task_31::run` itself reports `fail` at `"seam"` (the design's
//! last-writer-wins fallback, documented as the finding); that assertion
//! is folded into the last test to keep the 2V/2A count.

use phlow_gauntlet::tasks::task_31;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_31::CaseReport {
    let report = task_31::run_case(case)
        .unwrap_or_else(|e| panic!("task-31: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-31 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-31-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-31 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-31 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; two acknowledged writes to the
/// same record field lose the first silently — last-writer-wins,
/// documented as the finding (the design's named fallback).
#[test]
fn record_mutations_are_last_writer_wins() {
    assert_eq!(task_31::ID, "task-31");
    assert_eq!(task_31::NAME, "optimistic concurrency");
    assert_eq!(task_31::KIND, TaskKind::Rust);
    assert_eq!(
        task_31::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("record_mutations_are_last_writer_wins");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["writes_acknowledged"], 2,
        "both writes must be acknowledged:\n{joined}"
    );
    assert_eq!(
        report.metrics["writes_surviving"], 1,
        "only the last write survives:\n{joined}"
    );
    assert_eq!(
        report.metrics["rejections"], 0,
        "no write is rejected — there is no version check:\n{joined}"
    );
    assert!(
        joined.contains("silently lost"),
        "evidence must document the silent loss:\n{joined}"
    );
}

/// V: the generation pin never bumps across a write — the "version"
/// does not increment, so there is no version 8 / version 9 and no
/// retry-with-fresh-read path.
#[test]
fn publish_generation_pin_never_bumps() {
    let report = run_case("publish_generation_pin_never_bumps");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["generation_before"], report.metrics["generation_after"],
        "the generation must not change across a write:\n{joined}"
    );
    assert!(
        joined.contains("stale-handle guard"),
        "evidence must name what the pin actually is:\n{joined}"
    );
}

// --- adversarial ---

/// A: a writer that ignores the version field (wrong generation) is
/// rejected with `StaleGeneration` naming expected vs actual — and the
/// rejected write is never applied blind: the node stays non-terminal
/// with no digest recorded.
#[test]
fn ignored_generation_rejected_naming_expected_vs_got() {
    let report = run_case("ignored_generation_rejected_with_expected_vs_got");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["stale_rejected"], 1,
        "the stale write must be rejected:\n{joined}"
    );
    assert_eq!(
        report.metrics["blind_applications"], 0,
        "nothing may be applied blind:\n{joined}"
    );
    assert!(
        joined.contains("StaleGeneration"),
        "evidence must name the rejection error:\n{joined}"
    );
    assert!(
        joined.contains("expected"),
        "evidence must show expected vs actual are named:\n{joined}"
    );
}

/// A: a second publish for the same node is rejected (`DuplicateResult`)
/// — the first digest stands, the loser's intent is rejected rather than
/// silently applied. Folded in: the task-level `run` reports `fail` at
/// `"seam"` (no versioned CAS write path; LWW documented), keeping the
/// 2V/2A count.
#[test]
fn second_publish_rejected_and_task_fails_at_seam() {
    let report = run_case("second_publish_rejected_never_overwritten");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["duplicates_rejected"], 1,
        "the second publish must be rejected:\n{joined}"
    );
    assert!(
        joined.contains("digest-first"),
        "evidence must show the first digest survived:\n{joined}"
    );
    match task_31::run(&test_ctx()) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(where_, "seam", "task-31 must fail at the absent seam");
            let joined = evidence.join("\n");
            assert!(
                joined.contains("last-writer-wins"),
                "evidence must document the LWW finding:\n{joined}"
            );
            assert!(
                how.contains("no versioned compare-and-swap"),
                "the 'how' must name the absent mechanism: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-31 passed: a versioned CAS write path was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
