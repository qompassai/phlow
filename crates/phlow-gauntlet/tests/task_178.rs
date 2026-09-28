// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-178 (ephemeral bridge launch).
//!
//! Two validation cases against the scripted double (ScriptedPort +
//! fixture bridge child, deterministic, fast) plus the real-chromium
//! integration half on primo: launch opens exactly one DevTools
//! target, MCP initialize succeeds over stdio, and task end leaves
//! zero lingering targets and zero trace of the bridge PID.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_178;

fn check_case(case: &str) -> CaseReport {
    let report = task_178::run_case(case)
        .unwrap_or_else(|e| panic!("task-178 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-178 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: exactly one Target.created, one live target, MCP initialize ok.
#[test]
fn launch_one_target_initialize() {
    assert_eq!(task_178::ID, "task-178");
    let report = check_case("launch_one_target_initialize");
    let m = &report.metrics;
    assert_eq!(m["target_created_events"].as_u64().unwrap(), 1);
    assert_eq!(m["targets_closed"].as_u64().unwrap(), 1);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("protocolVersion 2024-11-05"),
        "evidence must show the initialize handshake:\n{joined}"
    );
}

/// V2: after task end the port goes 1 -> 0 targets and the bridge PID
/// vanishes from the process census.
#[test]
fn task_end_zero_lingering() {
    let report = check_case("task_end_zero_lingering");
    let m = &report.metrics;
    assert!(m["reaped"].as_bool().unwrap());
    assert_eq!(m["targets_closed"].as_u64().unwrap(), 1);
    assert!(m["census_clean"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("census_clean=true"),
        "evidence must show the census check:\n{joined}"
    );
}

/// Integration: real chromium on primo — open one target, close it,
/// kill the browser, assert zero lingering targets and no surviving
/// process. Skips cleanly where no chromium binary exists.
#[test]
fn real_chromium_integration() {
    let report = check_case("real_chromium_integration");
    let m = &report.metrics;
    let backend = m["backend"].as_str().unwrap();
    if backend == "skipped-no-chromium" {
        eprintln!("task-178 integration skipped: no chromium binary");
        return;
    }
    assert_eq!(backend, "real-chromium");
    assert_eq!(m["targets_opened"].as_u64().unwrap(), 1);
    assert_eq!(m["targets_after_close"].as_u64().unwrap(), 0);
    assert_eq!(m["targets_closed"].as_u64().unwrap(), 1);
    assert!(m["reaped"].as_bool().unwrap());
    assert!(m["census_clean"].as_bool().unwrap());
}
