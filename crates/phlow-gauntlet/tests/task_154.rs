// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-154 (versioned envelope routing).
//!
//! Three validation cases: v1 and v2 envelopes interleaved on one
//! stream are each routed to their version with matching-version
//! responses and the connection up throughout; version 0 gives
//! `VersionError::TooOld` and a missing version gives
//! `VersionError::Missing` while the connection stays up; the adapted
//! wire module carries the maddada attribution.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_154;

fn check_case(case: &str) -> CaseReport {
    let report = task_154::run_case(case)
        .unwrap_or_else(|e| panic!("task-154 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-154 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: interleaved v1/v2 → each routed to its version, responses
/// carry the matching version, connection up.
#[test]
fn interleaved_versions_routed() {
    let report = check_case("interleaved_versions_routed");
    let m = &report.metrics;
    assert_eq!(m["routed"].as_array().unwrap().len(), 6);
    assert_eq!(m["responses"].as_u64().unwrap(), 6);
    assert!(m["connection_up"].as_bool().unwrap());
}

/// V2: version 0 → TooOld, missing version → Missing, connection
/// stays up and usable.
#[test]
fn too_old_and_missing_typed() {
    let report = check_case("too_old_and_missing_typed");
    let m = &report.metrics;
    assert!(m["too_old_typed"].as_bool().unwrap());
    assert!(m["missing_typed"].as_bool().unwrap());
    assert_eq!(m["rejected"].as_u64().unwrap(), 2);
    assert_eq!(m["received"].as_u64().unwrap(), 1);
    assert!(m["connection_up"].as_bool().unwrap());
}

/// License gate: src/wire.rs and the task driver carry the maddada
/// attribution + source commit.
#[test]
fn license_header_present() {
    let report = check_case("license_header_present");
    assert_eq!(report.metrics["files_checked"].as_u64().unwrap(), 2);
}
