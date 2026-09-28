//! Integration tests for task-24 (priority preemption).
//!
//! The seam is ABSENT: phlow has no run-priority or preemption support —
//! a case-insensitive scan of every non-gauntlet `crates/*/src` tree finds
//! zero 'priorit'/'preempt' hits, `SchedulerLimits`/`NodeParams` carry no
//! priority field, and `cancel_run` is terminal with no resume path. The
//! four cases pin that honest finding against phlow's real scheduler
//! sources (compiled in via `#[path]`): 2 validation, 2 adversarial.
//!
//! Each test compiles its own scenario binary into a unique workdir
//! (pid + process-local counter): parallel tests never share a binary.
//! `task_24::run` itself reports `fail` with `where = "seam"`; that
//! assertion is folded into the last test to keep the 2V/2A count.

use phlow_gauntlet::tasks::task_24;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-local sequence so concurrent tests never share a workdir.
static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Compile the scenario binary into a unique scratch dir.
fn scenario_binary(case: &str) -> PathBuf {
    let seq = WORKDIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let work_dir = std::env::temp_dir().join(format!(
        "gauntlet-task-24-{case}-{}-{seq}",
        std::process::id()
    ));
    std::fs::create_dir_all(&work_dir)
        .unwrap_or_else(|e| panic!("task-24: cannot create workdir: {e}"));
    task_24::ensure_scenario_binary(&work_dir)
        .unwrap_or_else(|e| panic!("task-24: scenario compile failed for '{case}': {e}"))
}

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(binary: &Path, case: &str) -> task_24::CaseReport {
    let report = task_24::run_case(binary, case)
        .unwrap_or_else(|e| panic!("task-24: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-24 case '{case}' failed: {:?}",
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
        std::env::temp_dir().join(format!("gauntlet-task-24-run-{}-{seq}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-24 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-24 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; admitting a "low-priority" node
/// before a "high-priority" one keeps both Admitted in FIFO order —
/// admission is priority-blind because no priority API exists.
#[test]
fn admission_is_priority_blind() {
    assert_eq!(task_24::ID, "task-24");
    assert_eq!(task_24::NAME, "priority preemption");
    assert_eq!(task_24::KIND, TaskKind::Rust);
    assert_eq!(
        task_24::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let binary = scenario_binary("admission_is_priority_blind");
    let report = run_case(&binary, "admission_is_priority_blind");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("FIFO order kept"),
        "evidence must show FIFO admission order:\n{joined}"
    );
    assert!(
        joined.contains("no API exists to express or honor priority"),
        "evidence must state the priority API is absent:\n{joined}"
    );
}

/// V: the scheduler's tuning surface (`SchedulerLimits`) carries no
/// priority, preemption, quantum, or aging knob — the full field set is
/// enumerated in the evidence.
#[test]
fn limits_carry_no_priority_knob() {
    let binary = scenario_binary("limits_have_no_priority");
    let report = run_case(&binary, "limits_have_no_priority");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no priority, preemption, quantum, or aging knob"),
        "evidence must enumerate the missing knobs:\n{joined}"
    );
}

// --- adversarial ---

/// A: `cancel_run` is terminal — a cancelled run cannot be resumed with
/// state intact, so preemption's "resume" half does not exist. A late
/// publish for the cancelled node is refused (`RunCancelled`).
#[test]
fn cancellation_is_terminal_no_resume_path() {
    let binary = scenario_binary("cancellation_is_terminal");
    let report = run_case(&binary, "cancellation_is_terminal");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("terminal state with no resume path"),
        "evidence must show cancellation is terminal:\n{joined}"
    );
    assert!(
        joined.contains("RunCancelled"),
        "the late publish must be refused as RunCancelled:\n{joined}"
    );
}

/// A: a cancelled run id stays poisoned monotonically — re-admit and
/// re-cancel are refused, so no preempt/un-preempt cycle exists to starve
/// or age within. Folded in: the task-level `run` reports the evidenced
/// seam absence (`where = "seam"`), keeping the 2V/2A count.
#[test]
fn cancelled_run_stays_poisoned_and_task_reports_seam() {
    let binary = scenario_binary("cancelled_run_stays_poisoned");
    let report = run_case(&binary, "cancelled_run_stays_poisoned");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("poisoned monotonically"),
        "evidence must show the id stays poisoned:\n{joined}"
    );
    match task_24::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-24 must fail at the absent seam");
            assert!(
                how.contains("no run-priority or preemption support"),
                "the 'how' must name the absent capability: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-24 passed: the priority seam was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
