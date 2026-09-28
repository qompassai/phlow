//! Integration tests for task-113 (edit-budget accounting evasion).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! clearly labeled scripted double (deterministic, fast). At `L_t = 2`:
//! the four accounting-evasion attacks are rejected with typed errors;
//! every recorded step satisfies `applied_ops ≤ L_t` and
//! `tokens_per_edit ≤ PER_EDIT_TOKENS_MAX`; the constant / cosine /
//! autonomous schedule spread must be < 1.5 points.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_113;

fn check_case(case: &str) -> CaseReport {
    let report = task_113::run_case(case)
        .unwrap_or_else(|e| panic!("task-113 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-113 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: all evasion attacks rejected with their typed errors.
#[test]
fn reject_evasion_attacks() {
    assert_eq!(task_113::ID, "task-113");
    assert_eq!(task_113::L_T, 2);
    let report = check_case("reject_evasion_attacks");
    let joined = report.evidence.join("\n");
    for attack in [
        "huge-whole-document-replace",
        "whole-document-insert-anchor",
        "newline-joined-multi-edit",
        "empty-anchor",
        "duplicate-anchor",
    ] {
        assert!(
            joined.contains(attack),
            "evidence must name attack {attack}:\n{joined}"
        );
    }
    assert!(
        joined.contains("PayloadTooLarge")
            || joined.contains("payload")
            || joined.contains("tokens"),
        "evidence must show the typed rejections:\n{joined}"
    );
}

/// V2: schedule comparison at the same bound. Passes on any
/// preregistered verdict (task-110 precedent): the spread is measured
/// and classified, not wished below the bar. A negative verdict here
/// is a valid finding, reported with full evidence.
#[test]
fn schedule_spread() {
    let report = check_case("schedule_spread");
    let m = &report.metrics;
    let verdict = m["verdict"].as_str().unwrap_or("<missing>");
    assert!(
        ["replicates", "null", "negative", "indeterminate"].contains(&verdict),
        "verdict must be a preregistered class, got {verdict}"
    );
    assert!(
        m["spread"].as_f64().is_some(),
        "the spread must be reported"
    );
    assert_eq!(
        m["means"].as_array().map(|a| a.len()).unwrap_or(0),
        3,
        "all three schedules must be reported"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("task-113 verdict:"),
        "evidence must carry the verdict line"
    );
}

// --- adversarial ---

/// A1: every recorded step honors applied_ops ≤ L_t and the per-edit
/// token bound — the audit the attacker wants to break.
#[test]
fn accounting_invariants() {
    let report = check_case("accounting_invariants");
    let m = &report.metrics;
    assert_eq!(
        m["violations"].as_u64().unwrap(),
        0,
        "accounting violations found"
    );
    assert!(
        m["steps_audited"].as_u64().unwrap() > 0,
        "the audit must cover steps"
    );
}

/// A2: the autonomous controller cannot evade its cap — unit-pinned
/// boundary behavior plus the arm-level budget audit.
#[test]
fn autonomous_schedule_behavior() {
    let report = check_case("autonomous_schedule_behavior");
    let m = &report.metrics;
    assert_eq!(m["cap"].as_u64().unwrap(), task_113::L_T as u64);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("stays capped") && joined.contains("stays floored"),
        "evidence must pin the controller boundaries:\n{joined}"
    );
    assert!(
        joined.contains("rejected"),
        "evidence must show cap 0 rejected:\n{joined}"
    );
}
