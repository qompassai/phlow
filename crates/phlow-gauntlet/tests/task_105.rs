//! Integration tests for task-105 (evidence-size robustness).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised here
//! against the clearly-labeled scripted offline control (deterministic,
//! fast). All four are expected to PASS: the adversarial cases probe
//! whether the cells are stable enough for the spread claim to mean
//! anything, and whether the fraction study varies only the evidence
//! diet. The primary evidence run (task-level `run()`) uses the real
//! [`ModelOptimizer`] via primo's Ollama; in this sandbox it resolves to
//! the scripted fallback and must still Pass.
//!
//! Pass criteria are preregistered in the driver: replicates = grid
//! spread <2.0 D_test points AND 100%−10% ≥5.0 points AND the fraction
//! means monotonic (10≤50≤100); null = spread ≥2.0 OR the curve
//! flat/inverted.

use phlow_gauntlet::tasks::task_105;

// --- validation ---

/// V1: all 6 grid cells + 3 fraction arms run and the preregistered
/// verdict classifies.
#[test]
fn grid_complete_and_classified() {
    let report = task_105::run_case("grid_complete_and_classified")
        .unwrap_or_else(|e| panic!("task-105 case failed to run: {e}"));
    assert!(
        report.passed,
        "grid and fractions must run: {}",
        report.failures.join("; ")
    );
    let verdict = report.metrics["verdict"].as_str().unwrap_or("<missing>");
    assert!(
        ["replicates", "null", "indeterminate"].contains(&verdict),
        "verdict must be a preregistered class, got {verdict}"
    );
    assert_eq!(
        report.metrics["cells"].as_array().map(|a| a.len()),
        Some(6),
        "six grid cells must be reported"
    );
    assert_eq!(
        report.metrics["fractions"].as_array().map(|a| a.len()),
        Some(3),
        "three fraction arms must be reported"
    );
}

/// V2: the 100%−10% evidence-size gain clears the preregistered +5.0
/// points. (Full monotonicity does not replicate under the mock —
/// 50% < 10% on n=3 — which is why the grid verdict is null.)
#[test]
fn evidence_size_gain_replicates() {
    let report = task_105::run_case("evidence_size_gain_replicates")
        .unwrap_or_else(|e| panic!("task-105 case failed to run: {e}"));
    assert!(
        report.passed,
        "the evidence-size gain must replicate: {}",
        report.failures.join("; ")
    );
    assert!(
        report.metrics["gain"].as_f64().unwrap() >= 5.0,
        "the reported gain must clear 5 points"
    );
}

// --- adversarial ---

/// A1: adversarial to the design's robustness claim — with the scripted
/// mock, B_m dominates the grid (the mock can only fix failures it is
/// shown), so the <2.0-point spread is untestable with the double and
/// needs the real model. B_m=4 must beat B_m=1 at every B.
#[test]
fn reflection_minibatch_drives_mock() {
    let report = task_105::run_case("reflection_minibatch_drives_mock")
        .unwrap_or_else(|e| panic!("task-105 case failed to run: {e}"));
    assert!(
        report.passed,
        "the mock's B_m dependence must be measurable: {}",
        report.failures.join("; ")
    );
    for g in report.metrics["gaps"].as_array().unwrap() {
        assert!(
            g["gap"].as_f64().unwrap() > 0.0,
            "B_m=4 must beat B_m=1 at B={}",
            g["b"]
        );
    }
}

/// A2: D_sel/D_test are identical across the three fraction arms (same
/// s_0 scores), so only the evidence diet varies.
#[test]
fn fractions_share_eval_splits() {
    let report = task_105::run_case("fractions_share_eval_splits")
        .unwrap_or_else(|e| panic!("task-105 case failed to run: {e}"));
    assert!(
        report.passed,
        "eval splits must be shared across fractions: {}",
        report.failures.join("; ")
    );
}
