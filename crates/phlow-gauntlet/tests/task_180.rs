// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-180 (DevTools round trip).
//!
//! Three validation cases against the scripted DevTools endpoint
//! (canned Runtime.evaluate result objects, deterministic, fast) plus
//! the real-chromium integration half on primo: evaluate
//! document.title returns the exact title string typed; nested
//! objects map to MCP content without loss; undefined becomes a typed
//! null, not a crash.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_180;

fn check_case(case: &str) -> CaseReport {
    let report = task_180::run_case(case)
        .unwrap_or_else(|e| panic!("task-180 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-180 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: document.title -> the exact string, typed, untruncated.
#[test]
fn evaluate_title_typed() {
    assert_eq!(task_180::ID, "task-180");
    let report = check_case("evaluate_title_typed");
    assert_eq!(
        report.metrics["title"].as_str().unwrap(),
        "Wave 29 Test Page"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("round-tripped exactly"),
        "evidence must show the exact round trip:\n{joined}"
    );
}

/// V2: nested object -> MCP text -> parse == source value (no loss);
/// bigint arrives via unserializableValue.
#[test]
fn nested_object_mapped() {
    let report = check_case("nested_object_mapped");
    assert!(report.metrics["round_trip_lossless"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("byte-identical value"),
        "evidence must show lossless mapping:\n{joined}"
    );
    assert!(
        joined.contains("9007199254740993"),
        "evidence must show the bigint mapping:\n{joined}"
    );
}

/// undefined (and unknown expressions) -> typed Null, never a crash.
#[test]
fn undefined_is_null() {
    let report = check_case("undefined_is_null");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("typed Null"),
        "evidence must show the typed null:\n{joined}"
    );
}

/// Integration: real chromium opens a data URL and the exact title is
/// observed on the debugging port. Skips cleanly with no chromium.
#[test]
fn real_chromium_title_roundtrip() {
    let report = check_case("real_chromium_title_roundtrip");
    let backend = report.metrics["backend"].as_str().unwrap();
    if backend == "skipped-no-chromium" {
        eprintln!("task-180 integration skipped: no chromium binary");
        return;
    }
    assert_eq!(backend, "real-chromium");
    assert_eq!(
        report.metrics["title"].as_str().unwrap(),
        "Wave 29 Test Page"
    );
    assert!(report.metrics["census_clean"].as_bool().unwrap());
}
