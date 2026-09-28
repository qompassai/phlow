//! Integration tests for task-32 (event-sourced replay).
//!
//! The seam is HALF-ABSENT: the append path exists
//! (`EvaluationRecord::record_event`, bounded at EVENTS_MAX), but there
//! is no state-fold function and no replay entry point — events are
//! opaque strings (no typed event enum, no per-type fold cases) and
//! `to_json` is one-way (no `from_json`). The four cases drive the real
//! record (no mocks): 2 validation, 2 adversarial.
//!
//! `task_32::run` itself reports `fail` at `"seam"`; that assertion is
//! folded into the last test to keep the 2V/2A count.

use phlow_gauntlet::tasks::task_32;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_32::CaseReport {
    let report = task_32::run_case(case)
        .unwrap_or_else(|e| panic!("task-32: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-32 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-32-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-32 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-32 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; the append half of the seam
/// works — three scripted events append and are retained in order.
#[test]
fn events_append_and_are_retained() {
    assert_eq!(task_32::ID, "task-32");
    assert_eq!(task_32::NAME, "event-sourced replay");
    assert_eq!(task_32::KIND, TaskKind::Rust);
    assert_eq!(
        task_32::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("events_append_and_are_retained");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["events_appended"], 3,
        "three events must append:\n{joined}"
    );
    assert_eq!(
        report.metrics["event_count"], 3,
        "all three must be retained:\n{joined}"
    );
}

/// V: the logged events are opaque strings — zero event types, zero
/// fold cases. The design's "every event type has a fold case" criterion
/// is vacuous: there are no types to be exhaustive over.
#[test]
fn events_are_opaque_strings_no_fold_cases() {
    let report = run_case("events_are_opaque_strings_no_fold_cases");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["event_types"], 0,
        "there must be no typed events:\n{joined}"
    );
    assert_eq!(
        report.metrics["fold_cases"], 0,
        "there must be no fold cases:\n{joined}"
    );
    assert!(
        joined.contains("opaque string"),
        "evidence must state the events are opaque:\n{joined}"
    );
}

// --- adversarial ---

/// A: there is no replay entry point — `to_json` is one-way (no
/// `from_json`, no replay constructor, no fold function), so a replay
/// cannot be constructed against any real API. Hand-rolling one would
/// invent the seam, not drive it.
#[test]
fn no_replay_entry_point() {
    let report = run_case("no_replay_entry_point");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["replay_entry_points"], 0,
        "there must be no replay entry point:\n{joined}"
    );
    assert!(
        joined.contains("no from_json"),
        "evidence must state the serialization is one-way:\n{joined}"
    );
    assert!(
        joined.contains("inventing the seam"),
        "evidence must refuse to hand-roll a fold:\n{joined}"
    );
}

/// A: a mid-stream schema bump has no replay handler — `schema_version`
/// is a compile-time constant and no code path interprets a foreign
/// schema version, because there is no replay to run it against.
/// Folded in: the task-level `run` reports `fail` at `"seam"`, keeping
/// the 2V/2A count.
#[test]
fn schema_bump_has_no_handler_and_task_fails_at_seam() {
    let report = run_case("schema_bump_has_no_replay_handler");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["bump_handlers"], 0,
        "there must be no bump handler:\n{joined}"
    );
    assert!(
        joined.contains("compile-time constant"),
        "evidence must state the version is a constant:\n{joined}"
    );
    match task_32::run(&test_ctx()) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(where_, "seam", "task-32 must fail at the absent seam");
            let joined = evidence.join("\n");
            assert!(
                joined.contains("fold function"),
                "evidence must name the missing fold:\n{joined}"
            );
            assert!(
                how.contains("half-absent"),
                "the 'how' must state which half is missing: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => {
            panic!("task-32 passed: a replay fold was invented, not found\nevidence: {evidence:?}")
        }
    }
}
