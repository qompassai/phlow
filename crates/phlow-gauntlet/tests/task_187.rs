// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-187 (ranked fuzzy queries).
//!
//! Two driver cases against the scripted 200-session corpus (FsLog +
//! synthetic fixtures): the three-word title match is rank 1, and 10
//! repeated queries produce byte-identical rankings.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_187;

fn check_case(case: &'static str) -> CaseReport {
    let report = task_187::run_case(case)
        .unwrap_or_else(|e| panic!("task-187 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-187 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: "pairing brute force" → the session titled with those words is
/// rank 1, with distractors present so the ranking is exercised.
#[test]
fn target_ranks_first() {
    assert_eq!(task_187::ID, "task-187");
    let report = check_case("target_ranks_first");
    let m = &report.metrics;
    assert_eq!(m["rank_1"].as_str().unwrap(), "target-pairing-brute-force");
    assert!(
        m["hits"].as_u64().unwrap() >= 2,
        "distractors must survive the prefilter for a real ranking"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("pairing brute force"),
        "evidence must name the query:\n{joined}"
    );
}

/// V2: 10 runs of the same query → byte-identical rankings, i.e. no
/// nondeterministic tie-breaking.
#[test]
fn rankings_byte_identical() {
    let report = check_case("rankings_byte_identical");
    let m = &report.metrics;
    assert_eq!(m["runs"].as_u64().unwrap(), 10);
    assert_eq!(
        m["identical"].as_u64().unwrap(),
        10,
        "all 10 rankings must be byte-identical"
    );
}
