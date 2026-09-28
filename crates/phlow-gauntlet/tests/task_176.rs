// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-176 (sidecar supervision under failure).
//!
//! Two validation cases with scripted POSIX shell fixtures: a
//! crash-loop sidecar is restarted exactly `MAX_SIDECAR_RESTARTS`
//! times then parked in `Failed`, while the daemon's loopback
//! pairing never goes down; a SIGKILL mid-remote-pairing fails typed
//! as `SidecarDown` inside the no-hang bound, and loopback pairing
//! still works afterwards.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_176;

fn check_case(case: &str) -> CaseReport {
    let report = task_176::run_case(case)
        .unwrap_or_else(|e| panic!("task-176 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-176 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: crash-loop parks after exactly the bounded restart count;
/// loopback unaffected.
#[test]
fn crash_loop_parks_bounded() {
    assert_eq!(task_176::ID, "task-176");
    let report = check_case("crash_loop_parks_bounded");
    let m = &report.metrics;
    assert_eq!(m["restarts"].as_u64().unwrap(), 3);
    assert_eq!(m["restart_bound"].as_u64().unwrap(), 3);
    assert!(m["loopback_ok"].as_bool().unwrap());
    assert!(
        m["status"].as_str().unwrap().contains("Failed"),
        "supervisor must park in Failed, got {}",
        m["status"].as_str().unwrap()
    );
}

/// V2: SIGKILL mid-pairing → typed SidecarDown inside the bound;
/// loopback still pairs.
#[test]
fn sigkill_mid_pairing_typed() {
    let report = check_case("sigkill_mid_pairing_typed");
    let m = &report.metrics;
    assert_eq!(m["mid_pairing_failure"].as_str().unwrap(), "SidecarDown");
    assert!(m["loopback_ok"].as_bool().unwrap());
}
