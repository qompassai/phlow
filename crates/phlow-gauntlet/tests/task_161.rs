// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-161 (snapshot resync after reconnect).
//!
//! Two driver cases against the deterministic [`ClientState`] +
//! scripted daemon doubles (deterministic, fast): V1 asserts 5 missed
//! events converge via snapshot with deep-equal state and a clean
//! ledger; V2 asserts the snapshot racing live events buffers until the
//! `as_of` marker, applies in order with zero gaps/dupes, and rejects
//! a stale snapshot replay with state untouched.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_161;

fn check_case(case: &str) -> CaseReport {
    let report = task_161::run_case(case)
        .unwrap_or_else(|e| panic!("task-161 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-161 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: 5 missed events → snapshot applied, live resumes, deep-equal
/// state, zero duplicates in the ledger.
#[test]
fn missed_events_snapshot_resync() {
    assert_eq!(task_161::ID, "task-161");
    let report = check_case("missed_events_snapshot_resync");
    let m = &report.metrics;
    assert_eq!(m["applied_through"].as_u64().unwrap(), 7);
    assert_eq!(m["version"].as_u64().unwrap(), 7);
    assert!(
        m["deep_equal"].as_bool().unwrap(),
        "client state must deep-equal daemon state"
    );
    assert_eq!(m["dup_dropped"].as_u64().unwrap(), 0);
}

/// V2: racing live events are buffered until the snapshot's `as_of`,
/// then applied in order — ledger [(5,snapshot),(6,applied),(7,applied)];
/// the replayed event is dup-dropped and the stale snapshot is
/// rejected with state untouched.
#[test]
fn snapshot_races_live_events() {
    let report = check_case("snapshot_races_live_events");
    let m = &report.metrics;
    let ledger: Vec<(u64, &str)> = m["race_ledger"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["seq"].as_u64().unwrap(),
                e["disposition"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        ledger,
        vec![
            (6, "buffered"),
            (7, "buffered"),
            (5, "snapshot"),
            (6, "applied"),
            (7, "applied"),
            (8, "applied"),
            (6, "dup-dropped")
        ],
        "racing events buffer, the snapshot applies, the buffer drains in order, \
         live 8 applies, the replayed 6 is dup-dropped"
    );
    assert_eq!(m["dup_dropped"].as_u64().unwrap(), 1);
    assert!(m["stale_rejected"].as_bool().unwrap());
    assert!(m["deep_equal"].as_bool().unwrap());
}
