// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-183 (cancelled bridge reaping).
//!
//! Two adversarial cases: cancel mid-Runtime.evaluate (the scripted
//! port spins on the shared cancel flag) -> the in-flight evaluate
//! fails typed as Cancelled and everything is reaped within
//! BRIDGE_KILL_TIMEOUT; and a SIGTERM-ignoring bridge child ->
//! SIGKILL escalation, reaped, zero zombies in the process census.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_183;

fn check_case(case: &str) -> CaseReport {
    let report = task_183::run_case(case)
        .unwrap_or_else(|e| panic!("task-183 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-183 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: mid-evaluate cancel -> typed Cancelled, reaped within the
/// timeout, zero targets on the port, no zombie.
#[test]
fn cancel_mid_evaluate_reaps() {
    assert_eq!(task_183::ID, "task-183");
    let report = check_case("cancel_mid_evaluate_reaps");
    let m = &report.metrics;
    assert!(m["reaped"].as_bool().unwrap());
    assert_eq!(m["targets_remaining"].as_u64().unwrap(), 0);
    assert!(!m["zombie"].as_bool().unwrap());
    assert!(
        m["elapsed_ms"].as_u64().unwrap() < 5_000,
        "teardown must finish within BRIDGE_KILL_TIMEOUT"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("BridgeError::Cancelled"),
        "evidence must show the typed cancellation:\n{joined}"
    );
}

/// A2: SIGTERM-ignoring child -> escalated=true, reaped, no zombie.
#[test]
fn sigterm_ignored_escalates() {
    let report = check_case("sigterm_ignored_escalates");
    let m = &report.metrics;
    assert!(m["escalated"].as_bool().unwrap());
    assert!(m["reaped"].as_bool().unwrap());
    assert!(!(m["zombie"].as_bool().unwrap()));
    assert_eq!(m["targets_remaining"].as_u64().unwrap(), 0);
    assert!(
        m["elapsed_ms"].as_u64().unwrap() < 5_000,
        "escalated teardown must finish within BRIDGE_KILL_TIMEOUT"
    );
}
