//! Integration tests for task-139 (finding dedup across cycles).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! scaffold's `FindingStore` with synthetic findings (MOCK). The same
//! finding in two cycles is one record with two observations; distinct
//! findings are two records; near-duplicates are not fuzzy-merged; a
//! forced fingerprint collision is classified NEGATIVE — the scaffold
//! has no collision tiebreak, and that gap is measured, not wished
//! away.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_139;

fn check_case(case: &str) -> CaseReport {
    let report = task_139::run_case(case)
        .unwrap_or_else(|e| panic!("task-139 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-139 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: repeat observation — one record, observation_count == 2.
#[test]
fn repeat_observation_single_record() {
    assert_eq!(task_139::ID, "task-139");
    let report = check_case("repeat_observation_single_record");
    let m = &report.metrics;
    assert_eq!(m["records"].as_u64().unwrap(), 1);
    assert_eq!(m["observations"].as_u64().unwrap(), 2);
    assert!(!m["record_id"].as_str().unwrap().is_empty());
}

/// V2: two genuinely different findings — two records.
#[test]
fn distinct_findings_two_records() {
    let report = check_case("distinct_findings_two_records");
    assert_eq!(report.metrics["records"].as_u64().unwrap(), 2);
}

// --- adversarial ---

/// A1: near-duplicate (same title, different fingerprint) — two records,
/// no fuzzy merge.
#[test]
fn near_duplicate_no_fuzzy_merge() {
    let report = check_case("near_duplicate_no_fuzzy_merge");
    let m = &report.metrics;
    assert_eq!(m["records"].as_u64().unwrap(), 2);
    assert!(!m["fuzzy_merge"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no fuzzy merge"),
        "evidence must state the precision rule:\n{joined}"
    );
}

/// A2: forced fingerprint collision — verdict NEGATIVE. The scaffold
/// keys solely on the fingerprint string and drops the colliding
/// finding's content; the test pins the negative so a future scaffold
/// tiebreak trips it loudly.
#[test]
fn fingerprint_collision_preserves_data() {
    let report = check_case("fingerprint_collision_preserves_data");
    let m = &report.metrics;
    assert_eq!(
        m["verdict"].as_str().unwrap(),
        "negative",
        "the collision arm must classify negative until the scaffold gains a tiebreak"
    );
    assert!(!m["finding_b_preserved"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("task-139 verdict: negative"),
        "evidence must carry the verdict line:\n{joined}"
    );
}
