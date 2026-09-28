// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-185 (no persistent MCP config).
//!
//! Three validation cases: a bridge run leaves the MCP config dir
//! byte-identical (empty diff); the registry double shows no bridge
//! entry; 10 sequential tasks leave the surface byte-identical with
//! zero accumulation.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_185;

fn check_case(case: &str) -> CaseReport {
    let report = task_185::run_case(case)
        .unwrap_or_else(|e| panic!("task-185 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-185 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: config dir byte-identical after a bridge run (empty diff).
#[test]
fn config_dir_byte_identical() {
    assert_eq!(task_185::ID, "task-185");
    let report = check_case("config_dir_byte_identical");
    let m = &report.metrics;
    assert!(m["diff_empty"].as_bool().unwrap());
    assert_eq!(m["files"].as_u64().unwrap(), 2);
}

/// The registry keeps its pre-existing entry and gains no bridge entry.
#[test]
fn registry_clean() {
    let report = check_case("registry_clean");
    let m = &report.metrics;
    assert_eq!(m["registry_entries"].as_u64().unwrap(), 1);
    assert!(!m["bridge_entry"].as_bool().unwrap());
}

/// V2: 10 sequential tasks -> still byte-identical, registry empty.
#[test]
fn ten_sequential_no_accumulation() {
    let report = check_case("ten_sequential_no_accumulation");
    let m = &report.metrics;
    assert_eq!(m["runs"].as_u64().unwrap(), 10);
    assert!(m["diff_empty"].as_bool().unwrap());
    assert_eq!(m["registry_entries"].as_u64().unwrap(), 0);
}
