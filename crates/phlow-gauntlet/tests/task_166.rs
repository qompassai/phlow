// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-166 (pure transition function).
//!
//! Two validation cases against [`phlow_gauntlet::state_machine`]:
//! a scripted 200-event session replays byte-identically, and the
//! reducer module is statically asserted to import zero I/O
//! facilities.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_166;

fn check_case(case: &str) -> CaseReport {
    let report = task_166::run_case(case)
        .unwrap_or_else(|e| panic!("task-166 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-166 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: the 200-event scripted session runs twice; final states and
/// effect lists are byte-identical, and non-trivial (25 tasks,
/// version 200).
#[test]
fn replay_deterministic_200_events() {
    assert_eq!(task_166::ID, "task-166");
    let report = check_case("replay_deterministic_200_events");
    let m = &report.metrics;
    assert_eq!(m["events"].as_u64().unwrap(), 200);
    assert!(
        m["states_byte_identical"].as_bool().unwrap(),
        "final states must be byte-identical across replays"
    );
    assert!(
        m["effects_byte_identical"].as_bool().unwrap(),
        "effect lists must be byte-identical across replays"
    );
    assert_eq!(m["final_version"].as_u64().unwrap(), 200);
    assert_eq!(m["final_tasks"].as_u64().unwrap(), 25);
    assert!(
        m["effects_bytes"].as_u64().unwrap() > 0,
        "byte-equality is vacuous without recorded effects"
    );
}

/// V2: the source scan of src/state_machine.rs finds zero I/O
/// tokens — the reducer cannot perform I/O, not merely promises not
/// to.
#[test]
fn reducer_imports_no_io() {
    let report = check_case("reducer_imports_no_io");
    let m = &report.metrics;
    let hits = m["hits"].as_array().unwrap();
    assert!(
        hits.is_empty(),
        "I/O tokens found in the reducer module: {hits:?}"
    );
    assert!(m["reduce_defined"].as_bool().unwrap());
}
