//! Integration tests for task-104 (slow/meta update ablation).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised here
//! against the clearly-labeled scripted offline control (deterministic,
//! fast). All four are expected to PASS: the adversarial cases probe
//! whether the retention collapse and the D_sel cross-check are
//! *observable in the records*. The primary evidence run (task-level
//! `run()`) uses the real [`ModelOptimizer`] via primo's Ollama; in this
//! sandbox it resolves to the scripted fallback and must still Pass.
//!
//! Pass criteria are preregistered in the driver: replicates = neither
//! costs ≥8.0 D_test points vs full AND epoch-1 retention full ≥80% AND
//! neither <50%; null = neither within ±2.0 points of full.

use phlow_gauntlet::tasks::task_104;

// --- validation ---

/// V1: all four arms run and the preregistered verdict classifies.
#[test]
fn arms_complete_and_classified() {
    let report = task_104::run_case("arms_complete_and_classified")
        .unwrap_or_else(|e| panic!("task-104 case failed to run: {e}"));
    assert!(
        report.passed,
        "all four arms must run: {}",
        report.failures.join("; ")
    );
    let verdict = report.metrics["verdict"].as_str().unwrap_or("<missing>");
    assert!(
        ["replicates", "null", "indeterminate"].contains(&verdict),
        "verdict must be a preregistered class, got {verdict}"
    );
    assert_eq!(
        report.metrics["arms"].as_array().map(|a| a.len()),
        Some(4),
        "four arms must be reported"
    );
}

/// V2: the full arm's mean epoch-1 retention is ≥80% — the slow update
/// protects epoch-1 rules in this setup.
#[test]
fn full_arm_retains() {
    let report = task_104::run_case("full_arm_retains")
        .unwrap_or_else(|e| panic!("task-104 case failed to run: {e}"));
    assert!(
        report.passed,
        "the full arm must retain epoch-1 rules: {}",
        report.failures.join("; ")
    );
    assert!(
        report.metrics["retention"].as_f64().unwrap() >= 0.80,
        "the reported retention must clear 80%"
    );
}

// --- adversarial ---

/// A1: adversarial to the design's retention story — without slow/meta
/// protection the epoch-1 canonical rules must be visibly displaced.
/// Passes iff the collapse is measured (mean retention < 50%).
#[test]
fn neither_fails_retention() {
    let report = task_104::run_case("neither_fails_retention")
        .unwrap_or_else(|e| panic!("task-104 case failed to run: {e}"));
    assert!(
        report.passed,
        "the neither arm must show the retention collapse: {}",
        report.failures.join("; ")
    );
    assert!(
        report.metrics["retention"].as_f64().unwrap() < 0.50,
        "the reported neither retention must be below 50%"
    );
}

/// A2: the neither arm also loses on D_sel — guards against the verdict
/// being a D_test-split artifact.
#[test]
fn neither_loses_on_d_sel_too() {
    let report = task_104::run_case("neither_loses_on_d_sel_too")
        .unwrap_or_else(|e| panic!("task-104 case failed to run: {e}"));
    assert!(
        report.passed,
        "the neither arm must also lose on D_sel: {}",
        report.failures.join("; ")
    );
}
