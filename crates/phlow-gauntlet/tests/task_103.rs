//! Integration tests for task-103 (rejected-buffer ablation).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised here
//! against the clearly-labeled scripted offline control (deterministic,
//! fast). All four are expected to PASS: the adversarial cases probe
//! whether the buffer's mechanism (re-proposal suppression) and the
//! approval binding are *observable in the records*. The primary evidence
//! run (task-level `run()`) uses the real [`ModelOptimizer`] via primo's
//! Ollama; in this sandbox it resolves to the scripted fallback and must
//! still Pass.
//!
//! Pass criteria are preregistered in the driver: replicates = off costs
//! ≥1.5 D_test points vs full AND off re-proposal-within-3 ≥2× the full
//! rate AND write-only lands between on D_test; null = re-proposal rates
//! within ±20%.

use phlow_gauntlet::tasks::task_103;

// --- validation ---

/// V1: all three arms run and the preregistered verdict classifies.
#[test]
fn arms_complete_and_classified() {
    let report = task_103::run_case("arms_complete_and_classified")
        .unwrap_or_else(|e| panic!("task-103 case failed to run: {e}"));
    assert!(
        report.passed,
        "all three arms must run: {}",
        report.failures.join("; ")
    );
    let verdict = report.metrics["verdict"].as_str().unwrap_or("<missing>");
    assert!(
        ["replicates", "null", "indeterminate"].contains(&verdict),
        "verdict must be a preregistered class, got {verdict}"
    );
    assert_eq!(
        report.metrics["arms"].as_array().map(|a| a.len()),
        Some(3),
        "three arms must be reported"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("buffer hit rate"),
        "evidence must report the buffer hit rate:\n{joined}"
    );
}

/// V2: write-only D_test sits between full and off (inclusive).
#[test]
fn write_only_between() {
    let report = task_103::run_case("write_only_between")
        .unwrap_or_else(|e| panic!("task-103 case failed to run: {e}"));
    assert!(
        report.passed,
        "write-only must land between full and off: {}",
        report.failures.join("; ")
    );
}

// --- adversarial ---

/// A1: the off arm's re-proposal-within-3 rate must be ≥2× the full
/// arm's — the suppression mechanism must be visible, not just the
/// headline score.
#[test]
fn off_reproposes_more() {
    let report = task_103::run_case("off_reproposes_more")
        .unwrap_or_else(|e| panic!("task-103 case failed to run: {e}"));
    assert!(
        report.passed,
        "off must re-propose measurably more: {}",
        report.failures.join("; ")
    );
    assert!(
        report.metrics["holds"] == true,
        "the ≥2× re-proposal claim must hold in the metrics"
    );
    let full = report.metrics["full"].as_f64().unwrap();
    let off = report.metrics["off"].as_f64().unwrap();
    assert!(
        off >= 2.0 * full || (full == 0.0 && off > 0.0),
        "off ({off}) must re-propose ≥2× full ({full})"
    );
}

/// A2: an approval bound to one byte string does not export a mutated
/// skill; a fresh approval for the mutated bytes exports fine.
#[test]
fn approval_binding_holds() {
    let report = task_103::run_case("approval_binding_holds")
        .unwrap_or_else(|e| panic!("task-103 case failed to run: {e}"));
    assert!(
        report.passed,
        "stale approvals must not export: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["stale_refused"], true);
    assert_eq!(report.metrics["fresh_exported"], true);
}
