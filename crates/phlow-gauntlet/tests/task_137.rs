//! Integration tests for task-137 (cancellation without zombies).
//!
//! Four driver cases — 2 validation, 2 adversarial — against real
//! `sleep` children (the design demands real processes) with everything
//! else scripted (MOCK). Cancelling a queued target dequeues it with
//! zero processes; cancelling a running probe kills and reaps it with
//! no zombie left (verified by the absence of its /proc entry); a
//! terminal run refuses with typed `AlreadyTerminal`; a double cancel
//! kills exactly once.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_137;

fn check_case(case: &str) -> CaseReport {
    let report = task_137::run_case(case)
        .unwrap_or_else(|e| panic!("task-137 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-137 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: cancel a queued target — removed, zero processes spawned.
#[test]
fn cancel_queued_removes_without_spawn() {
    assert_eq!(task_137::ID, "task-137");
    let report = check_case("cancel_queued_removes_without_spawn");
    let m = &report.metrics;
    assert_eq!(m["outcome"].as_str().unwrap(), "dequeued");
    assert_eq!(m["spawned"].as_u64().unwrap(), 0);
    assert_eq!(
        m["queue_len"].as_u64().unwrap(),
        0,
        "cleanup must drain the queue"
    );
}

/// V2: cancel a running probe — kill + reap, no zombie (no /proc entry).
#[test]
fn cancel_running_reaps_no_zombie() {
    let report = check_case("cancel_running_reaps_no_zombie");
    let m = &report.metrics;
    assert_eq!(m["killed"].as_u64().unwrap(), 1);
    assert_eq!(m["reaped"].as_u64().unwrap(), 1);
    assert!(
        m["proc_entry_after_reap"].as_bool().unwrap(),
        "the reaped PID must have no /proc entry"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no zombie") || joined.contains("None = no zombie"),
        "evidence must state the zombie check:\n{joined}"
    );
}

// --- adversarial ---

/// A1: cancelling a Finished run is a typed AlreadyTerminal no-op.
#[test]
fn cancel_finished_is_already_terminal() {
    let report = check_case("cancel_finished_is_already_terminal");
    let m = &report.metrics;
    assert_eq!(m["error"].as_str().unwrap(), "AlreadyTerminal");
    assert_eq!(
        m["killed_before"].as_u64().unwrap(),
        m["killed_after"].as_u64().unwrap(),
        "a refused cancel must not kill"
    );
}

/// A2: double cancel — single kill, single reap, one ledger entry.
#[test]
fn double_cancel_single_kill() {
    let report = check_case("double_cancel_single_kill");
    let m = &report.metrics;
    assert_eq!(m["killed"].as_u64().unwrap(), 1, "exactly one kill");
    assert_eq!(m["reaped"].as_u64().unwrap(), 1, "exactly one reap");
    assert_eq!(m["cancelled_entries"].as_u64().unwrap(), 1);
    assert_eq!(m["second_cancel"].as_str().unwrap(), "AlreadyTerminal");
}
