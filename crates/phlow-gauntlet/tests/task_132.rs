//! Integration tests for task-132 (scope diffing: added/removed).
//!
//! Four driver cases — 2 validation, 2 adversarial — against
//! synthetic snapshots: v1={a,b,c} → v2={b,c,d,c'} yields
//! added={d}, removed={a}, changed=[(c,c')]; an empty new snapshot
//! removes everything without panicking; 10k-target snapshots diff
//! in under a second.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_132;

fn check_case(case: &str) -> CaseReport {
    let report = task_132::run_case(case)
        .unwrap_or_else(|e| panic!("task-132 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-132 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: exact set equality on added and removed.
#[test]
fn added_removed() {
    assert_eq!(task_132::ID, "task-132");
    let report = check_case("added_removed");
    let m = &report.metrics;
    let added: Vec<&str> = m["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let removed: Vec<&str> = m["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(added, vec!["d"]);
    assert_eq!(removed, vec!["a"]);
}

/// V2: the stable id with a changed value is exactly one `changed`
/// pair — never add+remove.
#[test]
fn changed_is_pair() {
    let report = check_case("changed_is_pair");
    let m = &report.metrics;
    assert_eq!(m["changed_count"].as_u64(), Some(1));
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("c: c.example.com -> c2.example.com"),
        "evidence must show the single changed pair:\n{joined}"
    );
}

// --- adversarial ---

/// A1: empty new snapshot → everything removed, nothing added, no
/// panic.
#[test]
fn empty_new_snapshot() {
    let report = check_case("empty_new_snapshot");
    let m = &report.metrics;
    let removed: Vec<&str> = m["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(removed, vec!["a", "b", "c"]);
    assert_eq!(m["added"].as_u64(), Some(0));
    assert_eq!(m["changed"].as_u64(), Some(0));
}

/// A2: 10k-target diff completes under the 1s wall budget with exact
/// counts (bounded O(n) work).
#[test]
fn ten_k_diff_bounded() {
    let report = check_case("ten_k_diff_bounded");
    let m = &report.metrics;
    let elapsed = m["elapsed_secs"].as_f64().unwrap();
    assert!(
        elapsed < task_132::DIFF_WALL_SECS_MAX,
        "10k diff took {elapsed:.3}s, budget {}s",
        task_132::DIFF_WALL_SECS_MAX
    );
    assert_eq!(m["added"].as_u64(), Some(50));
    assert_eq!(m["removed"].as_u64(), Some(50));
    assert_eq!(m["changed"].as_u64(), Some(100));
}
