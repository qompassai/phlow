// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-194 (idempotent skill sync).
//!
//! Two validation cases against temp-dir fixtures: syncing twice with
//! no changes between yields an empty second plan, zero writes, and
//! untouched mtimes; touching a target file without changing its
//! content still yields an empty plan — the content hash decides, not
//! the mtime.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_194;

fn check_case(case: &str) -> CaseReport {
    let report = task_194::run_case(case)
        .unwrap_or_else(|e| panic!("task-194 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-194 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: second sync with no changes → empty plan, zero fs mutations,
/// mtimes stable.
#[test]
fn double_sync_noop() {
    assert_eq!(task_194::ID, "task-194");
    let report = check_case("double_sync_noop");
    let m = &report.metrics;
    assert_eq!(m["plan_ops"].as_u64().unwrap(), 0);
    assert_eq!(m["fs_mutations"].as_u64().unwrap(), 0);
    assert!(m["mtimes_stable"].as_bool().unwrap());
}

/// V2: target file touched (mtime bumped, bytes identical) → plan
/// still empty, zero fs mutations — the hash, not the mtime, decides.
#[test]
fn touch_without_change_noop() {
    let report = check_case("touch_without_change_noop");
    let m = &report.metrics;
    assert_eq!(m["plan_ops"].as_u64().unwrap(), 0);
    assert_eq!(m["fs_mutations"].as_u64().unwrap(), 0);
    assert!(m["mtimes_stable"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("mtime before touch"),
        "evidence must show the touch actually bumped the mtime:\n{joined}"
    );
}
