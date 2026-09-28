//! Integration tests for task-107 (cross-family transfer).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised against
//! the clearly labeled scripted double (deterministic, fast). The
//! task measures **content portability, not skill discovery**: the
//! families are orthogonal by construction. V1 passes on any
//! preregistered verdict; A1 encodes the below-baseline-is-negative
//! rule.

use phlow_gauntlet::tasks::task_107;

// --- validation ---

/// V1: both transfer directions run and classify.
#[test]
fn transfer_classified() {
    let report = task_107::run_case("transfer_classified")
        .unwrap_or_else(|e| panic!("task-107 case failed to run: {e}"));
    assert!(
        report.passed,
        "both directions must run: {}",
        report.failures.join("; ")
    );
    for dir in ["FOrder->FBind", "FBind->FOrder"] {
        let verdict = report.metrics[dir]["verdict"]
            .as_str()
            .unwrap_or("<missing>");
        assert!(
            ["replicates", "null", "negative", "indeterminate"].contains(&verdict),
            "{dir}: verdict must be a preregistered class, got {verdict}"
        );
    }
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("task-107 verdict:"),
        "evidence must carry verdict lines:\n{joined}"
    );
    assert!(
        joined.contains("content portability, not skill discovery"),
        "evidence must state the scope:\n{joined}"
    );
}

/// V2: the source arm's own in-domain gain is positive — transfer
/// starts from real learning.
#[test]
fn in_domain_gains_positive() {
    let report = task_107::run_case("in_domain_gains_positive")
        .unwrap_or_else(|e| panic!("task-107 case failed to run: {e}"));
    assert!(
        report.passed,
        "in-domain gains must be positive: {}",
        report.failures.join("; ")
    );
}

// --- adversarial ---

/// A1: transferred D_test must never drop below the empty baseline —
/// below baseline is the negative finding.
#[test]
fn never_below_baseline() {
    let report = task_107::run_case("never_below_baseline")
        .unwrap_or_else(|e| panic!("task-107 case failed to run: {e}"));
    assert!(
        report.passed,
        "transfer must not go below baseline: {}",
        report.failures.join("; ")
    );
}

/// A2: the frozen-then-rebuilt document keeps Experiment provenance,
/// and untrusted text is still refused.
#[test]
fn provenance_preserved() {
    let report = task_107::run_case("provenance_preserved")
        .unwrap_or_else(|e| panic!("task-107 case failed to run: {e}"));
    assert!(
        report.passed,
        "provenance must survive the freeze: {}",
        report.failures.join("; ")
    );
}
