// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-181 (hostile page payload bounds).
//!
//! Four adversarial cases against hostile scripted page fixtures: a
//! 50 MB string is truncated at MAX_RESULT_BYTES with truncated:true;
//! a 512-deep object hits the depth bound during iterative
//! deserialization (no stack overflow); a cyclic value is rejected
//! per the declared contract; the allocation meter proves zero
//! unbounded allocations.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_181;

fn check_case(case: &str) -> CaseReport {
    let report = task_181::run_case(case)
        .unwrap_or_else(|e| panic!("task-181 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-181 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: 50 MB in, 64 KiB prefix out, truncated always signaled.
#[test]
fn fifty_mb_string_truncated() {
    assert_eq!(task_181::ID, "task-181");
    let report = check_case("fifty_mb_string_truncated");
    let m = &report.metrics;
    assert_eq!(m["input_bytes"].as_u64().unwrap(), 50 * 1024 * 1024);
    assert_eq!(m["kept_bytes"].as_u64().unwrap(), 65_536);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("truncated=true (signaled)"),
        "evidence must show signaled truncation:\n{joined}"
    );
    assert!(
        !joined.contains("SILENT TRUNCATION"),
        "truncation must never be silent:\n{joined}"
    );
}

/// A2a: 512-deep object -> depth bound, marker, no stack overflow.
#[test]
fn deep_object_depth_bound() {
    let report = check_case("deep_object_depth_bound");
    let m = &report.metrics;
    assert_eq!(m["input_depth"].as_u64().unwrap(), 512);
    assert_eq!(m["depth_bound"].as_u64().unwrap(), 32);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("depth marker present"),
        "evidence must show the depth marker:\n{joined}"
    );
}

/// A2b: cyclic value -> typed CyclicValue rejection per contract.
#[test]
fn cyclic_rejected() {
    let report = check_case("cyclic_rejected");
    assert_eq!(report.metrics["contract"].as_str().unwrap(), "reject");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("BridgeError::CyclicValue"),
        "evidence must name the typed rejection:\n{joined}"
    );
}

/// Every copied byte is metered and bounded; the cyclic value is
/// rejected before any copy.
#[test]
fn allocation_meter_bounded() {
    let report = check_case("allocation_meter_bounded");
    let bound = report.metrics["meter_bound"].as_u64().unwrap();
    assert_eq!(bound, 65_536 + 8 * 1024);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("rejected before copy"),
        "evidence must show zero-copy rejection:\n{joined}"
    );
}
