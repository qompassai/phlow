// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-188 (no per-agent parsers).
//!
//! Two driver cases: a static scan of the adapted engine module must
//! show zero foreign-agent markers, and a fake foreign-agent tree
//! dropped next to the session dir must see zero reads.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_188;

fn check_case(case: &'static str) -> CaseReport {
    let report = task_188::run_case(case)
        .unwrap_or_else(|e| panic!("task-188 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-188 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: the adapted engine references no foreign agent homes and no
/// per-agent parser module.
#[test]
fn no_foreign_agent_references() {
    assert_eq!(task_188::ID, "task-188");
    let report = check_case("no_foreign_agent_references");
    let m = &report.metrics;
    assert!(
        m["forbidden_hits"].as_array().unwrap().is_empty(),
        "forbidden markers found: {}",
        m["forbidden_hits"]
    );
    assert_eq!(m["markers_checked"].as_u64().unwrap(), 5);
}

/// V2: the fake foreign tree is ignored — zero reads under it, and
/// only phlow's own 5 sessions are indexed.
#[test]
fn fake_foreign_tree_ignored() {
    let report = check_case("fake_foreign_tree_ignored");
    let m = &report.metrics;
    assert_eq!(
        m["foreign_reads"].as_u64().unwrap(),
        0,
        "zero reads under the fake foreign tree"
    );
    assert_eq!(m["rows"].as_u64().unwrap(), 5);
}
