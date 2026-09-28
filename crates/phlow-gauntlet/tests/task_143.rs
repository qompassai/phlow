//! Integration tests for task-143 (false-positive rejection with
//! reason).
//!
//! Four driver cases — 2 validation, 2 adversarial — against scripted
//! fixtures (MOCK): rejection records its reason verbatim on the record
//! and in quarantine with evidence intact; the quarantine is queryable
//! for operator audit; an FP differing from a TP only in the evidence
//! field is rejected on exactly `evidence-present`; re-submitting a
//! rejected FP next cycle stays rejected (sticky by (fingerprint,
//! content-hash)).

use phlow_gauntlet::TaskKind;
use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_143;

fn check_case(case: &str) -> CaseReport {
    let report = task_143::run_case(case)
        .unwrap_or_else(|e| panic!("task-143 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-143 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: metadata contract pins the task; the rejection reason is
/// recorded verbatim and the finding lands Rejected in quarantine.
#[test]
fn rejection_records_reason() {
    assert_eq!(task_143::ID, "task-143");
    assert_eq!(task_143::NAME, "false-positive-rejection");
    assert_eq!(task_143::KIND, TaskKind::Rust);
    let report = check_case("rejection_records_reason");
    let m = &report.metrics;
    assert_eq!(m["state"].as_str().unwrap(), "Rejected");
    assert_eq!(m["reason"].as_str().unwrap(), "not-reproducible");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("evidence intact"),
        "evidence must state the quarantine kept the evidence:\n{joined}"
    );
}

/// V2: the quarantine lists every rejection with its verbatim reason,
/// in rejection order — the operator's audit surface.
#[test]
fn quarantine_queryable() {
    let report = check_case("quarantine_queryable");
    let m = &report.metrics;
    assert_eq!(m["quarantined"].as_u64().unwrap(), 2);
    let reasons: Vec<&str> = m["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["not-reproducible", "evidence-empty"]);
}

// --- adversarial ---

/// A1: the FP is identical to the TP except for the empty evidence
/// field, and the rejection names exactly `evidence-present`.
#[test]
fn empty_evidence_rejected() {
    let report = check_case("empty_evidence_rejected");
    let m = &report.metrics;
    assert_eq!(m["fp_check"].as_str().unwrap(), "evidence-present");
    assert!(m["tp_passed"].as_bool().unwrap());
}

/// A2: re-submitting the rejected FP next cycle creates no new record,
/// returns the original id, and the record stays Rejected with its
/// reason intact — rejection is sticky by (fingerprint, content-hash).
#[test]
fn rejection_sticky_across_cycles() {
    let report = check_case("rejection_sticky_across_cycles");
    let m = &report.metrics;
    assert!(!m["is_new"].as_bool().unwrap());
    assert_eq!(
        m["record_id"].as_str().unwrap(),
        m["resubmitted_id"].as_str().unwrap()
    );
    assert_eq!(m["state"].as_str().unwrap(), "Rejected");
    assert_eq!(m["reason"].as_str().unwrap(), "not-reproducible");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("sticky by (fingerprint, content-hash)"),
        "evidence must state the stickiness mechanism:\n{joined}"
    );
}
