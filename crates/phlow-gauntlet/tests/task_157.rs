// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-157 (version-skew games).
//!
//! Four adversarial cases: a v1 envelope after v3 negotiation is
//! rejected as `VersionError::Downgrade` and the session keeps its v3
//! mark; version 4,294,967,295 is rejected as `Unsupported`
//! pre-dispatch and version 0 as `TooOld` without touching session
//! state; the per-session mark moves only upward (1→2→3, then a
//! replayed 2 is a downgrade); the adapted wire module carries the
//! maddada attribution.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_157;

fn check_case(case: &str) -> CaseReport {
    let report = task_157::run_case(case)
        .unwrap_or_else(|e| panic!("task-157 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-157 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: v1 after v3 → Downgrade; session keeps v3 and stays usable.
#[test]
fn downgrade_rejected_session_intact() {
    let report = check_case("downgrade_rejected_session_intact");
    let m = &report.metrics;
    assert_eq!(m["downgrade_typed"].as_str().unwrap(), "Downgrade");
    assert_eq!(m["session_version"].as_u64().unwrap(), 3);
}

/// A2: version 2^32-1 → Unsupported pre-dispatch; version 0 →
/// TooOld; session untouched.
#[test]
fn absurd_version_rejected_pre_dispatch() {
    let report = check_case("absurd_version_rejected_pre_dispatch");
    let m = &report.metrics;
    assert_eq!(m["absurd_version"].as_u64().unwrap(), 4_294_967_295);
    assert_eq!(m["absurd_typed"].as_str().unwrap(), "Unsupported");
    assert_eq!(m["zero_typed"].as_str().unwrap(), "TooOld");
    assert!(m["session_untouched"].as_bool().unwrap());
}

/// The session mark moves only upward; a replayed v2 after 1→2→3 is
/// a downgrade.
#[test]
fn monotonic_tracking() {
    let report = check_case("monotonic_tracking");
    let m = &report.metrics;
    assert_eq!(m["mark"].as_u64().unwrap(), 3);
    assert_eq!(m["accepted"].as_u64().unwrap(), 3);
    assert!(m["downgrade_rejected"].as_bool().unwrap());
}

/// License gate: src/wire.rs and the task driver carry the maddada
/// attribution + source commit.
#[test]
fn license_header_present() {
    let report = check_case("license_header_present");
    assert_eq!(report.metrics["files_checked"].as_u64().unwrap(), 2);
}
