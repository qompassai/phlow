//! Integration tests for task-27 (retry backoff under storm).
//!
//! The seam is ABSENT: phlow's real downstream-call wrapper,
//! `phlow_llm::transport::OllamaBackend::chat`, has no retry/backoff /
//! jitter machinery — one transient failure is returned to the caller,
//! repeated explicit calls have no attempt cap, a storm reaches
//! downstream unthrottled, and no delay schedule is ever injected. The
//! four cases drive the real wrapper through a scripted implementation
//! of the real `LlmTransport` trait and pin that honest finding: 2
//! validation, 2 adversarial.
//!
//! Diver-owned finding (flagged, not modified): diver's
//! `lua/ai/harness/supervisor.lua` retry_run HAS exponential backoff
//! with jitter (RETRY_ATTEMPTS_MAX = 4) — a different repo's different
//! seam, out of scope for this Rust task.
//!
//! Each test drives `task_27::run_case` in-process (no scenario binary,
//! no workdir). `task_27::run` itself reports `fail` with
//! `where = "seam"`; that assertion is folded into the last test to keep
//! the 2V/2A count.

use phlow_gauntlet::tasks::task_27;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_27::CaseReport {
    let report = task_27::run_case(case)
        .unwrap_or_else(|e| panic!("task-27: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-27 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-27-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-27 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-27 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; a single transient failure is
/// NOT retried — the queued success reply is still waiting for an
/// explicit second call.
#[test]
fn transient_failure_is_not_retried() {
    assert_eq!(task_27::ID, "task-27");
    assert_eq!(task_27::NAME, "retry backoff under storm");
    assert_eq!(task_27::KIND, TaskKind::Rust);
    assert_eq!(
        task_27::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("transient_failure_is_not_retried");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["first_attempt_downstream_calls"], 1,
        "the transient failure must reach downstream exactly once:\\n{joined}"
    );
    assert!(
        joined.contains("did not retry"),
        "evidence must state the failure was not retried:\\n{joined}"
    );
}

/// V: many consecutive failures under explicit calls — every error is
/// the raw `LlmError::Transport`: no attempt cap engages and no typed
/// exhaustion error exists.
#[test]
fn no_attempt_cap_or_exhaustion_error() {
    let report = run_case("no_attempt_cap_or_exhaustion_error");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["downstream_calls"], report.metrics["calls"],
        "every explicit call must reach downstream:\\n{joined}"
    );
    assert!(
        joined.contains("no attempt cap engaged"),
        "evidence must state no cap engaged:\\n{joined}"
    );
    assert!(
        joined.contains("no typed exhaustion error exists"),
        "evidence must state no exhaustion type exists:\\n{joined}"
    );
}

// --- adversarial ---

/// A: a caller storm reaches the downstream unthrottled — no backoff, no
/// jitter, no cap: the path forwards the storm 1:1.
#[test]
fn storm_reaches_downstream_unthrottled() {
    let report = run_case("storm_reaches_downstream_unthrottled");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["downstream_calls"], report.metrics["storm_calls"],
        "the storm must reach downstream 1:1:\\n{joined}"
    );
    assert!(
        joined.contains("no backoff, no jitter, no cap"),
        "evidence must state the storm was unthrottled:\\n{joined}"
    );
}

/// A: per-call latency across consecutive failures is flat, not
/// exponential — no backoff schedule is injected anywhere in the path.
/// Folded in: the task-level `run` reports the evidenced seam absence
/// (`where = "seam"`), keeping the 2V/2A count.
#[test]
fn no_backoff_delays_injected_and_task_reports_seam() {
    let report = run_case("no_backoff_delays_injected");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("flat, not exponential"),
        "evidence must show the arrival times are flat:\\n{joined}"
    );
    match task_27::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-27 must fail at the absent seam");
            assert!(
                how.contains("no retry wrapper exists"),
                "the 'how' must name the absent machinery: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => {
            panic!("task-27 passed: the retry seam was invented, not found\nevidence: {evidence:?}")
        }
    }
}
