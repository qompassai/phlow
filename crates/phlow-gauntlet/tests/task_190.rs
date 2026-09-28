// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-190 (corrupt index rebuild).
//!
//! Two adversarial driver cases: a zeroed 4 KiB header and a
//! mid-table truncation both surface as typed IndexError::Corrupt,
//! quarantine the bad file, rebuild from a fresh scan, and serve
//! correct queries afterwards — never partial data from the corrupt
//! file.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_190;

fn check_case(case: &'static str) -> CaseReport {
    let report = task_190::run_case(case)
        .unwrap_or_else(|e| panic!("task-190 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-190 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: zeroed header → typed Corrupt, quarantine, rebuild; all 20
/// sessions queryable afterwards.
#[test]
fn zeroed_header_rebuilds() {
    assert_eq!(task_190::ID, "task-190");
    let report = check_case("zeroed_header_rebuilds");
    let m = &report.metrics;
    assert!(m["rebuilt"].as_bool().unwrap());
    assert!(m["quarantined"].as_bool().unwrap());
    assert_eq!(m["rows"].as_u64().unwrap(), 20);
    assert_eq!(m["queryable"].as_u64().unwrap(), 20);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("IndexError::Corrupt"),
        "evidence must show the typed corruption error:\n{joined}"
    );
}

/// A2: mid-table truncation → the same typed path within the scan
/// bound.
#[test]
fn truncated_midtable_rebuilds() {
    let report = check_case("truncated_midtable_rebuilds");
    let m = &report.metrics;
    assert!(m["rebuilt"].as_bool().unwrap());
    assert!(m["quarantined"].as_bool().unwrap());
    assert_eq!(m["rows"].as_u64().unwrap(), 20);
    assert_eq!(m["queryable"].as_u64().unwrap(), 20);
}
