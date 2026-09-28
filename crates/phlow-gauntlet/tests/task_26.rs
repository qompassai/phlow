//! Integration tests for task-26 (circuit breaker).
//!
//! The seam is ABSENT: phlow's real downstream-call wrapper,
//! `phlow_llm::transport::OllamaBackend::chat`, has no circuit-breaker /
//! half-open / fast-fail machinery — every call reaches `LlmTransport`,
//! failures included. The four cases drive the real wrapper through a
//! scripted implementation of the real `LlmTransport` trait and pin that
//! honest finding: 2 validation, 2 adversarial.
//!
//! Each test drives `task_26::run_case` in-process (no scenario binary,
//! no workdir). `task_26::run` itself reports `fail` with
//! `where = "seam"`; that assertion is folded into the last test to keep
//! the 2V/2A count.

use phlow_gauntlet::tasks::task_26;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_26::CaseReport {
    let report = task_26::run_case(case)
        .unwrap_or_else(|e| panic!("task-26: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-26 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-26-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-26 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-26 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; N consecutive downstream failures
/// each reach the downstream — nothing fast-fails, because there is no
/// breaker to open.
#[test]
fn consecutive_failures_all_reach_downstream() {
    assert_eq!(task_26::ID, "task-26");
    assert_eq!(task_26::NAME, "circuit breaker");
    assert_eq!(task_26::KIND, TaskKind::Rust);
    assert_eq!(
        task_26::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("consecutive_failures_all_reach_downstream");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["downstream_calls"], report.metrics["consecutive_failures"],
        "every consecutive failure must reach downstream:\\n{joined}"
    );
    assert!(
        joined.contains("no call fast-failed"),
        "evidence must show nothing fast-failed:\\n{joined}"
    );
}

/// V: recovery is per-call, not half-open — after failures, a scripted
/// success is served immediately by the very next call. There is no
/// half-open probe because there is no breaker state to probe.
#[test]
fn recovery_is_per_call() {
    let report = run_case("recovery_is_per_call");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["calls_before_recovery"], 3,
        "three failures must precede the recovery call:\\n{joined}"
    );
    assert_eq!(
        report.metrics["downstream_calls"], 4,
        "all four calls must reach downstream:\\n{joined}"
    );
    assert!(
        joined.contains("no half-open probe exists"),
        "evidence must state there is no half-open state:\\n{joined}"
    );
}

// --- adversarial ---

/// A: a 50-call failure storm is absorbed 1:1 — the honest cost of the
/// missing breaker, measured not asserted: a breaker would have capped
/// this at the open threshold.
#[test]
fn failure_storm_absorbed_one_to_one() {
    let report = run_case("failure_storm_absorbed_one_to_one");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["downstream_calls"], report.metrics["storm_calls"],
        "the storm must reach downstream 1:1, 0 fast-failed:\\n{joined}"
    );
    assert!(
        joined.contains("0 fast-failed"),
        "evidence must show no call was fast-failed:\\n{joined}"
    );
}

/// A: interleaved failures and successes — each call's outcome is
/// exactly its scripted reply, proving the path keeps no cross-call
/// state (no failure counter, no breaker state machine). Folded in: the
/// task-level `run` reports the evidenced seam absence
/// (`where = "seam"`), keeping the 2V/2A count.
#[test]
fn calls_are_stateless_and_task_reports_seam() {
    let report = run_case("calls_are_stateless");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["interleaved_calls"], 6,
        "six interleaved calls must each reach downstream:\\n{joined}"
    );
    assert_eq!(
        report.metrics["downstream_calls"], 6,
        "every interleaved call must reach downstream:\\n{joined}"
    );
    assert!(
        joined.contains("no cross-call state"),
        "evidence must state the path keeps no state:\\n{joined}"
    );
    match task_26::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-26 must fail at the absent seam");
            assert!(
                how.contains("no circuit breaker"),
                "the 'how' must name the absent breaker: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-26 passed: the circuit breaker seam was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
