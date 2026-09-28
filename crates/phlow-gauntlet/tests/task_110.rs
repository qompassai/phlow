//! Integration tests for task-110 (poisoned rollout evidence).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised against
//! the clearly labeled scripted double (deterministic, fast). The
//! double is NOT told the poisoning labels: it must fall for the
//! poisoned reflection evidence, and the buffer/gate containment is
//! what is measured. V1 passes on any preregistered verdict; the
//! preregistered rule is that any poison-tainted acceptance at 10% is
//! negative.

use phlow_gauntlet::tasks::task_110;

// --- validation ---

/// V1: all three poison arms run and the verdict classifies.
#[test]
fn poison_contained_classified() {
    let report = task_110::run_case("poison_contained_classified")
        .unwrap_or_else(|e| panic!("task-110 case failed to run: {e}"));
    assert!(
        report.passed,
        "all three arms must run: {}",
        report.failures.join("; ")
    );
    let verdict = report.metrics["verdict"].as_str().unwrap_or("<missing>");
    assert!(
        ["replicates", "null", "negative", "indeterminate"].contains(&verdict),
        "verdict must be a preregistered class, got {verdict}"
    );
    let levels = report.metrics["levels"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(levels.len(), 3, "three poison levels must be reported");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("task-110 verdict:"),
        "evidence must carry the verdict line:\n{joined}"
    );
}

/// V2: the optimizer must demonstrably fall for the poisoned evidence
/// (poison-induced considerations), and the buffer must record
/// rejections and suppress later re-proposals.
#[test]
fn buffer_records_and_helps() {
    let report = task_110::run_case("buffer_records_and_helps")
        .unwrap_or_else(|e| panic!("task-110 case failed to run: {e}"));
    assert!(
        report.passed,
        "buffer record-and-suppress must be observable: {}",
        report.failures.join("; ")
    );
}

// --- adversarial ---

/// A1: the 20% arm must not regress D_test vs the clean arm.
#[test]
fn no_regression_at_20() {
    let report = task_110::run_case("no_regression_at_20")
        .unwrap_or_else(|e| panic!("task-110 case failed to run: {e}"));
    assert!(
        report.passed,
        "D_test must not regress at 20% poison: {}",
        report.failures.join("; ")
    );
}

/// A2: no accepted edit at 10% may be both poison-induced AND harmful
/// to F-bind (non-order direction).
#[test]
fn no_harmful_poison_accepted() {
    let report = task_110::run_case("no_harmful_poison_accepted")
        .unwrap_or_else(|e| panic!("task-110 case failed to run: {e}"));
    assert!(
        report.passed,
        "no harmful poison-induced edit may be accepted: {}",
        report.failures.join("; ")
    );
}
