//! Integration tests for task-106 (self-improving curriculum dynamics).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised against
//! the clearly labeled scripted double (deterministic, fast). V1
//! passes on any preregistered verdict (null/negative are first-class
//! measurements); the adversarial cases assert the gate's core
//! invariants, which must hold.

use phlow_gauntlet::tasks::task_106;

// --- validation ---

/// V1: the composed arm runs to completion and classifies.
#[test]
fn composed_dynamics_classified() {
    let report = task_106::run_case("composed_dynamics_classified")
        .unwrap_or_else(|e| panic!("task-106 case failed to run: {e}"));
    assert!(
        report.passed,
        "the arm must run: {}",
        report.failures.join("; ")
    );
    let verdict = report.metrics["verdict"].as_str().unwrap_or("<missing>");
    assert!(
        ["replicates", "null", "negative", "indeterminate"].contains(&verdict),
        "verdict must be a preregistered class, got {verdict}"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("task-106 verdict:"),
        "evidence must carry the verdict line:\n{joined}"
    );
    assert!(
        joined.contains("scripted-double"),
        "evidence must label the scripted double:\n{joined}"
    );
}

/// V2: every step record carries a known veto label and the per-seed
/// training curves render.
#[test]
fn edit_ledger_legible() {
    let report = task_106::run_case("edit_ledger_legible")
        .unwrap_or_else(|e| panic!("task-106 case failed to run: {e}"));
    assert!(
        report.passed,
        "every step must carry a known veto label: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["seeds"], 5);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("d_sel curve"),
        "evidence must render per-seed curves:\n{joined}"
    );
}

// --- adversarial ---

/// A1: no step may regress D_sel — the strict gate's core invariant.
#[test]
fn never_regresses() {
    let report = task_106::run_case("never_regresses")
        .unwrap_or_else(|e| panic!("task-106 case failed to run: {e}"));
    assert!(
        report.passed,
        "D_sel must never regress: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["regressions"], 0);
}

/// A2: an untrusted document is refused before any rollout.
#[test]
fn untrusted_doc_cannot_enter_loop() {
    let report = task_106::run_case("untrusted_doc_cannot_enter_loop")
        .unwrap_or_else(|e| panic!("task-106 case failed to run: {e}"));
    assert!(
        report.passed,
        "untrusted doc must be refused: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["refused"], true);
}
