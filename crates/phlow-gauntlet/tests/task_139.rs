//! Integration tests for task-139 (finding dedup across cycles).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! scaffold's `FindingStore` with synthetic findings (MOCK). The same
//! finding in two cycles is one record with two observations; distinct
//! findings are two records; near-duplicates are not fuzzy-merged; a
//! forced fingerprint collision is resolved by the composite
//! (fingerprint, content_hash) tiebreak — both findings survive as
//! distinct records (verdict POSITIVE), while byte-identical
//! re-observations still dedup to one record.

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

/// A2: forced fingerprint collision — verdict POSITIVE. The
/// composite (fingerprint, content_hash) tiebreak stores both findings
/// as distinct records (both survive); a byte-identical re-observation
/// of A still dedups onto A's record instead of opening a third.
#[test]
fn fingerprint_collision_preserves_data() {
    let report = check_case("fingerprint_collision_preserves_data");
    let m = &report.metrics;
    assert_eq!(
        m["verdict"].as_str().unwrap(),
        "replicates",
        "the collision arm must classify positive: the tiebreak preserves both findings"
    );
    assert!(
        m["finding_b_preserved"].as_bool().unwrap(),
        "finding B's content must survive the collision"
    );
    assert_eq!(m["records"].as_u64().unwrap(), 2);
    assert!(
        m["duplicate_merged"].as_bool().unwrap(),
        "byte-identical duplicates must still dedup to one record"
    );
    let ids = m["record_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
    assert_ne!(
        ids[0].as_str().unwrap(),
        ids[1].as_str().unwrap(),
        "colliding findings must hold distinct record ids"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("task-139 verdict: replicates"),
        "evidence must carry the verdict line:\n{joined}"
    );
}
