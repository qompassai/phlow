//! Integration tests for task-109 (adversarial edit catch rate).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised against
//! the clearly labeled scripted double (the labeled adversarial mock,
//! deterministic, fast). V1 passes on any preregistered verdict; the
//! adversarial cases assert the defense invariants (no seed below s_0,
//! harmful single edits never accepted).

use phlow_gauntlet::tasks::task_109;

// --- validation ---

/// V1: the attacked arm runs, every edit is probed on held-out
/// splits, and the catch rate classifies.
#[test]
fn catch_rate_classified() {
    let report = task_109::run_case("catch_rate_classified")
        .unwrap_or_else(|e| panic!("task-109 case failed to run: {e}"));
    assert!(
        report.passed,
        "the probe must run: {}",
        report.failures.join("; ")
    );
    let verdict = report.metrics["verdict"].as_str().unwrap_or("<missing>");
    assert!(
        ["replicates", "null", "negative", "indeterminate"].contains(&verdict),
        "verdict must be a preregistered class, got {verdict}"
    );
    assert!(
        report.metrics["proposed"].as_u64().unwrap_or(0) > 0,
        "the attacker must propose edits"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("task-109 verdict:"),
        "evidence must carry the verdict line:\n{joined}"
    );
}

/// V2: every escape is named with its mechanism.
#[test]
fn escapes_named() {
    let report = task_109::run_case("escapes_named")
        .unwrap_or_else(|e| panic!("task-109 case failed to run: {e}"));
    assert!(
        report.passed,
        "the escape list must be complete: {}",
        report.failures.join("; ")
    );
}

// --- adversarial ---

/// A1: no seed's final D_test may drop below its s_0, even under attack.
#[test]
fn final_d_test_never_below_s0() {
    let report = task_109::run_case("final_d_test_never_below_s0")
        .unwrap_or_else(|e| panic!("task-109 case failed to run: {e}"));
    assert!(
        report.passed,
        "final D_test must never drop below s_0: {}",
        report.failures.join("; ")
    );
}

/// A2: a harmful edit proposed ALONE must never be accepted — the gate
/// sees its full delta.
#[test]
fn single_edits_always_caught() {
    let report = task_109::run_case("single_edits_always_caught")
        .unwrap_or_else(|e| panic!("task-109 case failed to run: {e}"));
    assert!(
        report.passed,
        "harmful single edits must always be caught: {}",
        report.failures.join("; ")
    );
}
