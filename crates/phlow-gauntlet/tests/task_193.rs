// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-193 (scan-plan-apply sync).
//!
//! Two validation cases against temp-dir fixtures driving the shared
//! scan → plan → apply pipeline: canonical {a,b} vs target {a(old),c}
//! produces exactly {update a, add b, remove c}, apply executes the
//! plan, the replaced `a` is backed up byte-identical; and a dry run
//! prints the plan with zero filesystem writes (fs-access log).

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_193;

fn check_case(case: &str) -> CaseReport {
    let report = task_193::run_case(case)
        .unwrap_or_else(|e| panic!("task-193 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-193 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: canonical {a(new), b} vs target {a(old), c} → plan exactly
/// {update a, add b, remove c}; 2 writes + 1 remove + 2 backups in the
/// fs log; backups byte-identical to the pre-sync versions.
#[test]
fn plan_apply_backup() {
    assert_eq!(task_193::ID, "task-193");
    let report = check_case("plan_apply_backup");
    let m = &report.metrics;
    assert_eq!(m["plan_ops"].as_u64().unwrap(), 3);
    assert_eq!(m["writes"].as_u64().unwrap(), 2);
    assert_eq!(m["removes"].as_u64().unwrap(), 1);
    assert_eq!(m["backups"].as_u64().unwrap(), 2);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("backup a.md sha256="),
        "evidence must confirm the byte-identical backup:\n{joined}"
    );
}

/// V2: dry-run → plan printed as ["update a.md"], zero fs mutations,
/// no backup dir created, target untouched.
#[test]
fn dry_run_writes_nothing() {
    let report = check_case("dry_run_writes_nothing");
    let m = &report.metrics;
    assert_eq!(m["fs_mutations"].as_u64().unwrap(), 0);
    let printed = m["plan_printed"].as_array().unwrap();
    assert_eq!(printed.len(), 1);
    assert_eq!(printed[0].as_str().unwrap(), "update a.md");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("(bar: 0)"),
        "evidence must state the zero-write bar:\n{joined}"
    );
}
