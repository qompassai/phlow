// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-174 (single-use code consumption).
//!
//! Two adversarial cases: a replayed code (same code, same secret)
//! is refused as `Consumed` with no second device registered and the
//! replayed secret buffer zeroized; two threads racing the same code
//! produce exactly one `PairingOk` and one `Consumed`, because the
//! whole verify sequence holds one mutex.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_174;

fn check_case(case: &str) -> CaseReport {
    let report = task_174::run_case(case)
        .unwrap_or_else(|e| panic!("task-174 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-174 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: replay refused as Consumed; device count unchanged; replayed
/// secret zeroized.
#[test]
fn replay_refused() {
    assert_eq!(task_174::ID, "task-174");
    let report = check_case("replay_refused");
    let m = &report.metrics;
    assert!(m["replay_refused_as_consumed"].as_bool().unwrap());
    assert_eq!(m["device_count"].as_u64().unwrap(), 1);
    assert!(m["replay_secret_zeroized"].as_bool().unwrap());
}

/// A2: the concurrent race yields exactly one winner and one
/// Consumed; exactly one device registers.
#[test]
fn concurrent_verify_one_winner() {
    let report = check_case("concurrent_verify_one_winner");
    let m = &report.metrics;
    assert_eq!(m["wins"].as_u64().unwrap(), 1);
    assert_eq!(m["consumed"].as_u64().unwrap(), 1);
    assert_eq!(m["device_count"].as_u64().unwrap(), 1);
}
