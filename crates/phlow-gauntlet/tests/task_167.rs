// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-167 (effect interpreter separation).
//!
//! Two validation cases against
//! [`phlow_gauntlet::state_machine::MockInterpreter`]: effects execute
//! in recorded order across the fs/notify mocks, and an unknown
//! effect kind rejects the whole batch atomically with a typed error.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_167;

fn check_case(case: &str) -> CaseReport {
    let report = task_167::run_case(case)
        .unwrap_or_else(|e| panic!("task-167 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-167 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: [Write, Notify, Write] executes in recorded order. The
/// cross-sink sequence — not just per-sink counts — must match
/// exactly.
#[test]
fn effects_execute_in_recorded_order() {
    assert_eq!(task_167::ID, "task-167");
    let report = check_case("effects_execute_in_recorded_order");
    let m = &report.metrics;
    let sequence: Vec<&str> = m["sequence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        sequence,
        [
            "fs:persist task 1",
            "fs:persist task 1",
            "fs:persist task 1",
            "notify:notify task 1: task 1 done",
            "fs:persist task 2",
            "fs:persist task 2",
        ],
        "application order must match the recorded order"
    );
    assert_eq!(m["fs_writes"].as_u64().unwrap(), 5);
    assert_eq!(m["notifies"].as_u64().unwrap(), 1);
}

/// V2: a batch containing an unknown effect is rejected atomically —
/// `EffectError::Unhandled`, zero side effects on any mock.
#[test]
fn unknown_effect_atomic_batch_reject() {
    let report = check_case("unknown_effect_atomic_batch_reject");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "Unhandled");
    assert_eq!(m["unknown_name"].as_str().unwrap(), "future-effect");
    assert_eq!(
        m["fs_writes_after_reject"].as_u64().unwrap(),
        0,
        "atomicity broken: a write applied before the rejection"
    );
    assert_eq!(
        m["notifies_after_reject"].as_u64().unwrap(),
        0,
        "atomicity broken: a notify applied before the rejection"
    );
    assert_eq!(m["atomicity"].as_str().unwrap(), "batch-reject");
}
