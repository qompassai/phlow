// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-152 (no deny_unknown_fields).
//!
//! Two validation cases: an envelope with 3 known + 5 unknown fields
//! parses under the permissive decoder, every unknown is dropped, and
//! a same-version resend re-parses; the static source scan finds zero
//! `deny_unknown_fields` attributes in the real `phlow-mcp` and
//! `phlow-runtime` sources; the adapted wire module carries the
//! maddada attribution.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_152;

fn check_case(case: &str) -> CaseReport {
    let report = task_152::run_case(case)
        .unwrap_or_else(|e| panic!("task-152 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-152 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: 3 known + 5 unknown fields → parses, known exact, unknown
/// dropped; the dropped count surfaces in the debug metric.
#[test]
fn unknown_fields_ignored() {
    let report = check_case("unknown_fields_ignored");
    let m = &report.metrics;
    assert!(m["parsed"].as_bool().unwrap());
    assert_eq!(m["known_fields"].as_u64().unwrap(), 3);
    assert_eq!(
        m["unknown_fields"].as_u64().unwrap(),
        5,
        "all 5 unknown fields must be counted in the debug metric"
    );
}

/// V2: the static scan reports zero deny_unknown_fields attributes
/// in phlow-mcp and phlow-runtime.
#[test]
fn no_deny_unknown_fields_in_sources() {
    let report = check_case("static_scan_no_deny");
    let m = &report.metrics;
    assert_eq!(m["deny_unknown_fields_hits"].as_u64().unwrap(), 0);
    assert!(
        m["files_scanned"].as_u64().unwrap() > 0,
        "the scan must cover actual source files"
    );
}

/// License gate: src/wire.rs and the task driver carry the maddada
/// attribution + source commit.
#[test]
fn license_header_present() {
    let report = check_case("license_header_present");
    assert_eq!(report.metrics["files_checked"].as_u64().unwrap(), 2);
}
