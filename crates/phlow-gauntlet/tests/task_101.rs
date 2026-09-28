//! Integration tests for task-101 (edit-budget bound ablation).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised here
//! against the clearly-labeled scripted offline control (deterministic,
//! fast). All four are expected to PASS: the adversarial cases probe
//! whether the bound/pathology are *observable in the records*, not
//! whether an attack succeeds. The primary evidence run (task-level
//! `run()`) uses the real [`ModelOptimizer`] via primo's Ollama; in this
//! sandbox it resolves to the scripted fallback and must still Pass.
//!
//! Pass criteria are preregistered in the driver: replicates = unbounded
//! costs ≥2.0 D_test points vs L_t=4 AND bounded-arm spread <2.0;
//! null = unbounded within ±1.0; negative = unbounded wins by ≥2.0.

use phlow_gauntlet::tasks::task_101;

// --- validation ---

/// V1: all four arms run and the preregistered verdict classifies.
/// PASSES on any verdict — null/negative are first-class measurements.
#[test]
fn arms_complete_and_classified() {
    let report = task_101::run_case("arms_complete_and_classified")
        .unwrap_or_else(|e| panic!("task-101 case failed to run: {e}"));
    assert!(
        report.passed,
        "all four arms must run: {}",
        report.failures.join("; ")
    );
    let verdict = report.metrics["verdict"].as_str().unwrap_or("<missing>");
    assert!(
        ["replicates", "null", "negative", "indeterminate"].contains(&verdict),
        "verdict must be a preregistered class, got {verdict}"
    );
    assert_eq!(
        report.metrics["arms"].as_array().map(|a| a.len()),
        Some(4),
        "four arms must be reported"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("task-101 verdict:"),
        "evidence must carry the verdict line:\n{joined}"
    );
}

/// V2: bounded arms never apply more than L_t edits on an accepted step.
#[test]
fn truncation_bound_holds() {
    let report = task_101::run_case("truncation_bound_holds")
        .unwrap_or_else(|e| panic!("task-101 case failed to run: {e}"));
    assert!(
        report.passed,
        "truncation bound must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["violations"], 0);
}

// --- adversarial ---

/// A1: the unbounded arm's mean churn per step must exceed L_t=1's —
/// the rewrite-everything pathology must be observable in the records.
#[test]
fn unbounded_churn_pathology() {
    let report = task_101::run_case("unbounded_churn_pathology")
        .unwrap_or_else(|e| panic!("task-101 case failed to run: {e}"));
    assert!(
        report.passed,
        "the unbounded pathology must be measurable: {}",
        report.failures.join("; ")
    );
    assert!(
        report.metrics["unbounded_churn_per_step"].as_f64().unwrap()
            > report.metrics["lt1_churn_per_step"].as_f64().unwrap(),
        "metrics must show the churn gap"
    );
}

/// A2: a document imported from untrusted text is refused before any
/// rollout — the loop only optimizes experiment-state documents.
#[test]
fn untrusted_doc_cannot_enter_loop() {
    let report = task_101::run_case("untrusted_doc_cannot_enter_loop")
        .unwrap_or_else(|e| panic!("task-101 case failed to run: {e}"));
    assert!(
        report.passed,
        "untrusted documents must be refused: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["refused"], true);
}
