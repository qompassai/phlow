// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-192 (shared index, concurrent readers).
//!
//! One validation case (8 readers × 100 queries: all correct, zero
//! lock errors in WAL mode, plus the wave-30 license audit) and one
//! adversarial case (rescan racing readers: every hit page is
//! single-generation, never torn).

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_192;

fn check_case(case: &'static str) -> CaseReport {
    let report = task_192::run_case(case)
        .unwrap_or_else(|e| panic!("task-192 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-192 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: 800 concurrent queries, all correct, zero errors; the license
/// audit over all 8 adapted files passes.
#[test]
fn concurrent_readers_no_lock_errors() {
    assert_eq!(task_192::ID, "task-192");
    let report = check_case("concurrent_readers_no_lock_errors");
    let m = &report.metrics;
    assert_eq!(
        m["correct"].as_u64().unwrap(),
        8 * 100,
        "every concurrent query must return the right top hit"
    );
    assert_eq!(m["errors"].as_u64().unwrap(), 0);
    assert_eq!(
        m["license_offenders"].as_u64().unwrap(),
        0,
        "every adapted file must carry the attribution header"
    );
}

/// A1: 2 rescans racing 8 × 200 queries → zero torn pages, zero wrong
/// top hits, zero errors; only pre/post generations observed.
#[test]
fn rescan_during_reads_no_torn_rows() {
    let report = check_case("rescan_during_reads_no_torn_rows");
    let m = &report.metrics;
    assert_eq!(m["torn_pages"].as_u64().unwrap(), 0);
    assert_eq!(m["wrong_top"].as_u64().unwrap(), 0);
    assert_eq!(m["errors"].as_u64().unwrap(), 0);
    assert_eq!(
        m["rescans_ok"].as_u64().unwrap(),
        2,
        "both writer rescans must complete — otherwise the race is vacuous"
    );
    assert!(
        m["generations_seen"].as_array().unwrap().len() <= 3,
        "readers must see only pre/post-rescan states: {}",
        m["generations_seen"]
    );
}
