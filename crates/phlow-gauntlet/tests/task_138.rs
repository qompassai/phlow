//! Integration tests for task-138 (restart recovery without rescan).
//!
//! Four driver cases — 2 validation, 2 adversarial — against a
//! file-backed ledger double (MOCK) in a per-case temp dir. Kill after
//! 3 of 10 finish: resume probes exactly the remaining 7. A corrupted
//! ledger refuses with typed `LedgerCorrupt` and probes nothing. A
//! Finished run with a lost blob is re-probed exactly once. Resume
//! under a new scope version intersects correctly.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_138;

fn check_case(case: &str) -> CaseReport {
    let report = task_138::run_case(case)
        .unwrap_or_else(|e| panic!("task-138 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-138 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: resume probes exactly the 7 unfinished targets; the ledger ends
/// with 10 Finished and the driver plan agrees with the scaffold.
#[test]
fn resume_probes_remaining_only() {
    assert_eq!(task_138::ID, "task-138");
    let report = check_case("resume_probes_remaining_only");
    let m = &report.metrics;
    assert_eq!(m["resumed_probes"].as_u64().unwrap(), 7);
    assert_eq!(m["skipped_finished"].as_u64().unwrap(), 3);
    assert_eq!(m["targets_covered"].as_u64().unwrap(), 10);
    assert!(m["scaffold_agrees"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("never rescanned"),
        "evidence must state completed targets are not rescanned:\n{joined}"
    );
}

/// V2: a corrupted ledger refuses with typed LedgerCorrupt; 0 probes.
#[test]
fn corrupt_ledger_refuses() {
    let report = check_case("corrupt_ledger_refuses");
    let m = &report.metrics;
    assert_eq!(
        m["probed"].as_u64().unwrap(),
        0,
        "fail closed: probe nothing"
    );
    let err = m["error"].as_str().unwrap();
    assert!(
        err.contains("LedgerError::Corrupt"),
        "the refusal must be typed, got: {err}"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no silent rescan"),
        "evidence must state the fail-closed rule:\n{joined}"
    );
}

// --- adversarial ---

/// A1: a Finished run with a lost blob is re-probed exactly once.
#[test]
fn lost_blob_reprobed_once() {
    let report = check_case("lost_blob_reprobed_once");
    let m = &report.metrics;
    let reprobed = m["reprobed"].as_array().unwrap();
    assert_eq!(reprobed.len(), 1);
    assert_eq!(reprobed[0].as_str().unwrap(), "t02");
    assert_eq!(m["t02_run_records"].as_u64().unwrap(), 2);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("at-least-once"),
        "evidence must name the at-least-once semantics:\n{joined}"
    );
}

/// A2: new scope version — completed ∩ new scope skipped, dropped never
/// re-probed, new queued.
#[test]
fn new_scope_intersection() {
    let report = check_case("new_scope_intersection");
    let m = &report.metrics;
    assert_eq!(m["probed"].as_u64().unwrap(), 9, "t04..t12");
    assert_eq!(m["skipped_finished"].as_u64().unwrap(), 1, "t03");
    assert_eq!(m["scope_version"].as_u64().unwrap(), 2);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("never re-probed"),
        "evidence must state dropped targets are not re-probed:\n{joined}"
    );
}
