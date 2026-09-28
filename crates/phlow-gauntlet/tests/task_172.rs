// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-172 (constant-time secret compare).
//!
//! Two validation cases: real accept/reject loops are timed and
//! their per-iteration times stay within a sane factor, the
//! instrumented compare visits exactly 32 bytes on match and on
//! mismatch, the caller's secret buffer is all-zero after verify on
//! both paths, and neither the store dump nor the audit log contains
//! the plaintext secret.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_172;

fn check_case(case: &str) -> CaseReport {
    let report = task_172::run_case(case)
        .unwrap_or_else(|e| panic!("task-172 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-172 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: measured accept/reject timing within the factor bound, and
/// the compare visits 32 bytes on both outcomes.
#[test]
fn timing_side_channel_bounded() {
    assert_eq!(task_172::ID, "task-172");
    let report = check_case("timing_side_channel_bounded");
    let m = &report.metrics;
    let factor = m["factor"].as_f64().unwrap();
    assert!(
        factor <= task_172::TIMING_FACTOR_BOUND,
        "accept/reject factor {factor} exceeds bound {}",
        task_172::TIMING_FACTOR_BOUND
    );
    assert_eq!(m["ct_match_visited"].as_u64().unwrap(), 32);
    assert_eq!(m["ct_mismatch_visited"].as_u64().unwrap(), 32);
}

/// V2: caller secret buffers zeroized; store and audit free of the
/// plaintext secret.
#[test]
fn secret_zeroized_everywhere() {
    let report = check_case("secret_zeroized_everywhere");
    let m = &report.metrics;
    assert!(m["accept_buffer_zero"].as_bool().unwrap());
    assert!(m["mismatch_buffer_zero"].as_bool().unwrap());
    assert!(!m["secret_in_store_or_audit"].as_bool().unwrap());
    assert!(m["hash_in_store"].as_bool().unwrap());
}
