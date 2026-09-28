//! Integration tests for task-102 (selection-gate ablation).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised here
//! against the clearly-labeled scripted offline control (deterministic,
//! fast). All four are expected to PASS: the adversarial cases probe
//! whether the gate's load-bearing role is *observable in the records*.
//! The primary evidence run (task-level `run()`) uses the real
//! [`ModelOptimizer`] via primo's Ollama; in this sandbox it resolves to
//! the scripted fallback and must still Pass.
//!
//! Pass criteria are preregistered in the driver: replicates = gate-off
//! final D_test ≤ s_0 in ≥3/5 seeds AND ≥30% of gate-off accepted edits
//! post-hoc neutral/harmful AND the tie arm accepts more zero-gain edits
//! than strict; null = gate-off still improves in ≥4/5 seeds.

use phlow_gauntlet::tasks::task_102;

// --- validation ---

/// V1: all three arms run and the preregistered verdict classifies.
#[test]
fn arms_complete_and_classified() {
    let report = task_102::run_case("arms_complete_and_classified")
        .unwrap_or_else(|e| panic!("task-102 case failed to run: {e}"));
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
}

/// V2: every accepted step in the strict arm has d_sel_after strictly
/// greater than d_sel_before.
#[test]
fn strict_accepts_only_gains() {
    let report = task_102::run_case("strict_accepts_only_gains")
        .unwrap_or_else(|e| panic!("task-102 case failed to run: {e}"));
    assert!(
        report.passed,
        "the strict gate must accept only gains: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["violations"], 0);
    assert!(
        report.metrics["checked"].as_u64().unwrap() > 0,
        "the strict arm must have accepted at least one step"
    );
}

// --- adversarial ---

/// A1: the gate-off arm must accept edits whose post-hoc ΔD_sel ≤ 0 —
/// edits the gate would have caught. If none exist, the gate is
/// unmeasurable in this setup and the case fails honestly.
#[test]
fn gate_off_accepts_harm() {
    let report = task_102::run_case("gate_off_accepts_harm")
        .unwrap_or_else(|e| panic!("task-102 case failed to run: {e}"));
    assert!(
        report.passed,
        "gate-off must accept measurable harm: {}",
        report.failures.join("; ")
    );
    assert!(
        report.metrics["harm_frac"].as_f64().unwrap() > 0.0,
        "the harm fraction must be positive"
    );
}

/// A2: longitudinal memory (KEEP/GUIDE/CYCLE lines) appears only in the
/// protected section, never in the body — the epoch-end writer cannot
/// leak into, and step edits cannot reach, the protected region.
#[test]
fn slow_update_only_writes_protected() {
    let report = task_102::run_case("slow_update_only_writes_protected")
        .unwrap_or_else(|e| panic!("task-102 case failed to run: {e}"));
    assert!(
        report.passed,
        "protected-section lines must not leak into the body: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["leaked"], 0);
}
