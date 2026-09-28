// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-196 (concurrent modification safety).
//!
//! Two adversarial cases: a target file modified after the scan but
//! before apply aborts with `SyncError::ConcurrentModification`
//! (zero writes, intruder bytes intact); an apply killed midway
//! (test-only hook) is recovered by a rerun that restores the
//! pre-sync state from backup and completes byte-equal to a clean
//! single apply.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_196;

fn check_case(case: &str) -> CaseReport {
    let report = task_196::run_case(case)
        .unwrap_or_else(|e| panic!("task-196 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-196 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- adversarial ---

/// A1: intruder edit between scan and apply → ConcurrentModification,
/// zero fs mutations, intruder bytes intact, b.md never added.
#[test]
fn concurrent_modification_aborts() {
    assert_eq!(task_196::ID, "task-196");
    let report = check_case("concurrent_modification_aborts");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "ConcurrentModification");
    assert_eq!(m["fs_mutations"].as_u64().unwrap(), 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("intruder bytes intact"),
        "evidence must confirm the target was untouched:\n{joined}"
    );
}

/// A2: killed apply → rerun restores a.md from backup (restore logged),
/// applies both ops, leaves no marker, and the final tree is
/// byte-equal to a clean single apply.
#[test]
fn kill_midway_recovers() {
    let report = check_case("kill_midway_recovers");
    let m = &report.metrics;
    assert_eq!(m["applied_on_rerun"].as_u64().unwrap(), 2);
    assert!(m["restores_logged"].as_u64().unwrap() >= 1);
    let restored = m["restored"].as_array().unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].as_str().unwrap(), "a.md");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("byte-equals a clean single apply"),
        "evidence must state the convergence bar:\n{joined}"
    );
}
