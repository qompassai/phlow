//! Integration tests for task-134 (passive→active approval gating).
//!
//! Four driver cases — 2 validation, 2 adversarial — against fixture
//! approvals: passive recon launches with no marker; an active probe
//! with a live operator approval bound to the current scope version
//! launches; a stale-scope approval is refused with
//! `GateError::ScopeVersionMismatch`; a forged marker (correct shape,
//! wrong issuer) is refused by provenance with `GateError::NoApproval`.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_134;

fn check_case(case: &str) -> CaseReport {
    let report = task_134::run_case(case)
        .unwrap_or_else(|e| panic!("task-134 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-134 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: passive recon needs no approval marker.
#[test]
fn passive_needs_no_approval() {
    assert_eq!(task_134::ID, "task-134");
    let report = check_case("passive_needs_no_approval");
    assert_eq!(
        report.metrics["passive_launched_without_approval"].as_bool(),
        Some(true)
    );
}

/// V2: active probe with a live operator approval bound to scope v2
/// launches, bound to the approval's nonce.
#[test]
fn active_with_bound_approval() {
    let report = check_case("active_with_bound_approval");
    assert_eq!(report.metrics["bound_nonce"].as_u64(), Some(42));
}

// --- adversarial ---

/// A1: approval bound to scope v1, scope now v2 → refused with
/// `GateError::ScopeVersionMismatch`.
#[test]
fn stale_scope_approval_refused() {
    let report = check_case("stale_scope_approval_refused");
    let m = &report.metrics;
    assert_eq!(m["refused"].as_bool(), Some(true));
    assert_eq!(m["error"].as_str(), Some("ScopeVersionMismatch"));
}

/// A2: forged marker (wrong issuer) refused by provenance, not shape;
/// wrong program likewise refused.
#[test]
fn forged_marker_refused() {
    let report = check_case("forged_marker_refused");
    let m = &report.metrics;
    assert_eq!(m["forged_refused"].as_bool(), Some(true));
    assert_eq!(m["wrong_program_refused"].as_bool(), Some(true));
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("NoApproval"),
        "evidence must name the provenance refusal:\n{joined}"
    );
}
