// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-165 (worker shutdown without zombies).
//!
//! Two driver cases against the real threaded [`DaemonClient`] over a
//! [`ScriptedLink`] double: V1 asserts an idle worker joins within 1 s
//! with fds back to baseline; A1 asserts 50 in-flight messages against
//! an unresponsive daemon complete within SHUTDOWN_TIMEOUT with one
//! logged disposition per message.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_165;
use std::sync::{Mutex, MutexGuard};

/// The worker-thread census is process-global, so the two tests in
/// this binary must not overlap — otherwise one test's live worker
/// thread pollutes another's post-shutdown census.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn check_case(case: &str) -> CaseReport {
    let report = task_165::run_case(case)
        .unwrap_or_else(|e| panic!("task-165 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-165 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: idle worker → joined, elapsed ≤ 1 s, fd delta 0, zero worker
/// threads afterwards.
#[test]
fn idle_shutdown_joins() {
    let _guard = serial();
    assert_eq!(task_165::ID, "task-165");
    let report = check_case("idle_shutdown_joins");
    let m = &report.metrics;
    assert!(m["joined"].as_bool().unwrap());
    assert!(
        m["elapsed_ms"].as_u64().unwrap() <= m["idle_join_bar_ms"].as_u64().unwrap(),
        "idle shutdown must join within the 1 s bar"
    );
    assert_eq!(m["fd_delta"].as_i64().unwrap(), 0);
    assert_eq!(m["worker_threads_after"].as_u64().unwrap(), 0);
}

// --- adversarial ---

/// A1: 50 in-flight vs an unresponsive daemon → shutdown completes
/// within SHUTDOWN_TIMEOUT, exactly one disposition per message (all
/// dropped, in order, reason WriteTimeout), zero worker threads after.
#[test]
fn unresponsive_daemon_fifty_inflight() {
    let _guard = serial();
    let report = check_case("unresponsive_daemon_fifty_inflight");
    let m = &report.metrics;
    assert_eq!(m["in_flight"].as_u64().unwrap(), 50);
    assert_eq!(m["dispositions"].as_u64().unwrap(), 50);
    assert!(
        m["all_dropped_in_order"].as_bool().unwrap(),
        "every message gets exactly one logged disposition"
    );
    assert!(
        m["elapsed_ms"].as_u64().unwrap() <= m["shutdown_timeout_ms"].as_u64().unwrap(),
        "shutdown must respect SHUTDOWN_TIMEOUT even with 50 in-flight messages"
    );
    assert!(m["joined"].as_bool().unwrap());
    assert_eq!(m["worker_threads_after"].as_u64().unwrap(), 0);
}
