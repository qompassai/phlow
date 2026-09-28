// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-191 (query literal handling).
//!
//! Two adversarial driver cases proving the query is data, not code:
//! `%` matches only literal-`%` sessions (escaped LIKE form in the
//! query log), and an injection-shaped query matches nothing while
//! the table stays untouched.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_191;

fn check_case(case: &'static str) -> CaseReport {
    let report = task_191::run_case(case)
        .unwrap_or_else(|e| panic!("task-191 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-191 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: `%` matches only sessions with a literal `%` — never the
/// whole table — and the log shows the escaped pattern.
#[test]
fn percent_matches_literally() {
    assert_eq!(task_191::ID, "task-191");
    let report = check_case("percent_matches_literally");
    let m = &report.metrics;
    assert_eq!(m["query"].as_str().unwrap(), "%");
    assert_eq!(
        m["hits"].as_u64().unwrap(),
        1,
        "only the literal-% session may match"
    );
    assert!(
        m["hits"].as_u64().unwrap() < m["total"].as_u64().unwrap(),
        "a wildcard leak would match everything"
    );
    assert!(m["escaped"].as_bool().unwrap());
}

/// A2: `" OR "1"="1` → zero matches, table untouched, escaped form
/// logged.
#[test]
fn or_injection_matches_literally() {
    let report = check_case("or_injection_matches_literally");
    let m = &report.metrics;
    assert_eq!(m["hits"].as_u64().unwrap(), 0);
    assert_eq!(
        m["rows_before"].as_u64().unwrap(),
        m["rows_after"].as_u64().unwrap(),
        "a query must never modify the table"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("\" OR \"1\"=\"1"),
        "evidence must show the escaped query form:\n{joined}"
    );
}
