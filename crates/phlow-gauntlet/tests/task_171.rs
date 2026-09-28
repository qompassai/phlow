// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-171 (pairing code TTL expiry).
//!
//! Two validation cases against the in-memory [`Daemon`] with a
//! [`ManualClock`]: the expiry boundary is exact at the TTL second
//! (T+899 s accepted, T+900 s and T+901 s refused as `Expired`), and
//! the purge drops 1,000 expired records inside a named time bound
//! while keeping live codes usable.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_171;

fn check_case(case: &str) -> CaseReport {
    let report = task_171::run_case(case)
        .unwrap_or_else(|e| panic!("task-171 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-171 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: T+899 s accepted; T+900 s and T+901 s refused as Expired.
#[test]
fn ttl_boundary() {
    assert_eq!(task_171::ID, "task-171");
    let report = check_case("ttl_boundary");
    let m = &report.metrics;
    assert!(m["accepted_at_899"].as_bool().unwrap());
    assert!(m["expired_at_900"].as_bool().unwrap());
    assert!(m["expired_at_901"].as_bool().unwrap());
    assert_eq!(m["ttl_secs"].as_u64().unwrap(), 900);
}

/// V2: 1,000 expired codes purged, 3 live kept, inside the bound.
#[test]
fn purge_expired_bounded() {
    let report = check_case("purge_expired_bounded");
    let m = &report.metrics;
    assert_eq!(m["purged"].as_u64().unwrap(), 1000);
    assert_eq!(m["remaining"].as_u64().unwrap(), 3);
    let purge_ms = m["purge_ms"].as_u64().unwrap();
    let bound_ms = m["purge_bound_ms"].as_u64().unwrap();
    assert!(
        purge_ms <= bound_ms,
        "purge took {purge_ms}ms, bound is {bound_ms}ms"
    );
}
