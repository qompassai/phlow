// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-189 (poisoned session index).
//!
//! Two adversarial driver cases: SQL metacharacters in every text
//! field are stored literally (the SqlLog template audit proves all
//! writes go through the single parameterized template), and a
//! 20 MiB field truncates at MAX_FIELD_BYTES with the flag set.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_189;

fn check_case(case: &'static str) -> CaseReport {
    let report = task_189::run_case(case)
        .unwrap_or_else(|e| panic!("task-189 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-189 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: `'); DROP TABLE sessions; --` in every field → stored
/// literally, table intact (row count 1), queryable as data; exactly
/// one parameterized write template was used.
#[test]
fn sql_metacharacters_stored_literally() {
    assert_eq!(task_189::ID, "task-189");
    let report = check_case("sql_metacharacters_stored_literally");
    let m = &report.metrics;
    assert_eq!(m["rows"].as_u64().unwrap(), 1);
    assert!(m["literal"].as_bool().unwrap());
    assert_eq!(
        m["write_templates"].as_u64().unwrap(),
        1,
        "all session writes must go through one parameterized template"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("DROP TABLE sessions"),
        "evidence must show the payload survived literally:\n{joined}"
    );
}

/// A2: 20 MiB field → stored at exactly MAX_FIELD_BYTES with the
/// truncation flag; the build completes.
#[test]
fn huge_field_truncated() {
    let report = check_case("huge_field_truncated");
    let m = &report.metrics;
    assert_eq!(m["input_bytes"].as_u64().unwrap(), 20 * 1_048_576);
    assert_eq!(
        m["stored_bytes"].as_u64().unwrap(),
        m["bound"].as_u64().unwrap(),
        "stored bytes must equal the truncation bound exactly"
    );
    assert!(m["truncated"].as_bool().unwrap());
}
