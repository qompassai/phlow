// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-173 (brute-force rate limiting).
//!
//! Two adversarial cases against the in-memory [`Daemon`] with a
//! [`ManualClock`]: the sixth wrong-secret attempt inside the window
//! is `RateLimited` without running the secret comparison (even the
//! correct secret is refused while the window holds), and a
//! 100-attempt storm pairs nothing, logs one audit entry per
//! violation, and leaves a sibling code's budget untouched.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_173;

fn check_case(case: &str) -> CaseReport {
    let report = task_173::run_case(case)
        .unwrap_or_else(|e| panic!("task-173 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-173 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: five mismatches, then the sixth attempt is RateLimited with
/// the comparison counter unmoved; the window then expires and the
/// code works again.
#[test]
fn sixth_attempt_blocked() {
    assert_eq!(task_173::ID, "task-173");
    let report = check_case("sixth_attempt_blocked");
    let m = &report.metrics;
    assert_eq!(m["mismatches"].as_u64().unwrap(), 5);
    assert!(m["blocked_as_rate_limited"].as_bool().unwrap());
    assert_eq!(
        m["comparisons_after_block"].as_u64().unwrap(),
        m["comparisons_before_block"].as_u64().unwrap(),
        "the blocked attempt must not run the secret comparison"
    );
    assert!(m["recovers_after_window"].as_bool().unwrap());
}

/// A2: 100 wrong-secret attempts — zero pairings, first RateLimited
/// at attempt six, audit entries labeled, sibling code unaffected.
#[test]
fn abuse_storm_no_pairing() {
    let report = check_case("abuse_storm_no_pairing");
    let m = &report.metrics;
    assert_eq!(m["storm_attempts"].as_u64().unwrap(), 100);
    assert_eq!(m["pairings"].as_u64().unwrap(), 0);
    assert_eq!(m["first_rate_limited_at"].as_u64().unwrap(), 6);
    assert_eq!(m["audit_violations"].as_u64().unwrap(), 95);
    assert!(m["sibling_fresh_budget"].as_bool().unwrap());
}
