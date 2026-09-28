// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-164 (half-open connection timeout).
//!
//! Two adversarial driver cases against the real loopback-TCP
//! [`DaemonFixture`]: A1 asserts a 3-byte header stall is reaped on the
//! read deadline with the slot, fd, and thread released; A2 asserts 100
//! simultaneous stallers are all reaped within the bound, the fd count
//! returns to baseline, and a legitimate client is accepted after.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_164;
use std::sync::{Mutex, MutexGuard};

/// The fd/thread census is process-global, so the two tests in this
/// binary must not overlap — otherwise one test's listener and accept
/// thread pollute the other's baseline.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn check_case(case: &str) -> CaseReport {
    let report = task_164::run_case(case)
        .unwrap_or_else(|e| panic!("task-164 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-164 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- adversarial ---

/// A1: one 3-byte stall → zero slots held, fd count unchanged, no
/// parked handler thread, read-timeout audit line.
#[test]
fn three_byte_stall_reaped() {
    let _guard = serial();
    assert_eq!(task_164::ID, "task-164");
    let report = check_case("three_byte_stall_reaped");
    let m = &report.metrics;
    assert_eq!(m["slots_held"].as_u64().unwrap(), 0);
    assert_eq!(
        m["fd_before"].as_u64().unwrap(),
        m["fd_after"].as_u64().unwrap(),
        "the stalled peer's fd must be closed"
    );
    assert_eq!(
        m["threads_before"].as_u64().unwrap(),
        m["threads_after"].as_u64().unwrap(),
        "no handler thread may be left parked on the stall"
    );
    assert!(m["read_timeout_logged"].as_bool().unwrap());
}

/// A2: 100 stallers → all reaped within FRAME_READ_TIMEOUT + 3 s, fd
/// count back to baseline, legitimate client accepted afterwards.
#[test]
fn hundred_stallers_reaped() {
    let _guard = serial();
    let report = check_case("hundred_stallers_reaped");
    let m = &report.metrics;
    assert_eq!(m["stallers"].as_u64().unwrap(), 100);
    assert_eq!(m["slots_held"].as_u64().unwrap(), 0);
    assert_eq!(
        m["fd_before"].as_u64().unwrap(),
        m["fd_after"].as_u64().unwrap(),
        "fd count must return to baseline after the stall storm"
    );
    assert!(
        m["legitimate_accepted"].as_bool().unwrap(),
        "the daemon must still serve a legitimate client after reaping 100 stallers"
    );
}
