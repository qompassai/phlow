// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-177 (pairing ceremony integration +
//! license audit).
//!
//! One validation case: the full legitimate ceremony — issue,
//! present within the TTL, exactly one device registered with its
//! label and pairing time, paired calls authenticate, unknown ids
//! are refused — and the recorded transcript replayed against a
//! fresh daemon fails every step with zero devices registered. One
//! adversarial case: every Wave 28 Ghostex-adapted Rust file opens
//! with the exact attribution header.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_177;

fn check_case(case: &str) -> CaseReport {
    let report = task_177::run_case(case)
        .unwrap_or_else(|e| panic!("task-177 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-177 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V: full ceremony registers exactly one device; the transcript
/// replay on a fresh daemon is refused everywhere.
#[test]
fn full_ceremony_registers_one() {
    assert_eq!(task_177::ID, "task-177");
    let report = check_case("full_ceremony_registers_one");
    let m = &report.metrics;
    assert_eq!(m["devices_registered"].as_u64().unwrap(), 1);
    assert!(m["paired_call_ok"].as_bool().unwrap());
    assert!(m["unknown_call_refused"].as_bool().unwrap());
    assert_eq!(m["replay_steps_refused"].as_u64().unwrap(), 1);
    assert_eq!(m["replay_devices"].as_u64().unwrap(), 0);
}

/// A: all 17 Wave 28 adapted files carry the exact header.
#[test]
fn license_headers_exact() {
    let report = check_case("license_headers_exact");
    let m = &report.metrics;
    assert_eq!(m["files_checked"].as_u64().unwrap(), 17);
    assert_eq!(m["files_exact"].as_u64().unwrap(), 17);
}
