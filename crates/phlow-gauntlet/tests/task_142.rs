//! Integration tests for task-142 (finding validation pipeline).
//!
//! Four driver cases — 2 validation, 2 adversarial — against scripted
//! fixtures (MOCK): all four mechanical checks pass -> the finding is
//! reportable; each check individually toggled to fail blocks with the
//! check named; a validator error fails closed (never "pass on error");
//! an operator-registered custom check runs in the pipeline without
//! weakening it.

use phlow_gauntlet::TaskKind;
use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_142;

fn check_case(case: &str) -> CaseReport {
    let report = task_142::run_case(case)
        .unwrap_or_else(|e| panic!("task-142 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-142 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: metadata contract pins the task; 4/4 checks pass and the state
/// machine admits Candidate -> Validated -> Reportable.
#[test]
fn all_checks_pass_reportable() {
    assert_eq!(task_142::ID, "task-142");
    assert_eq!(task_142::NAME, "finding-validation-pipeline");
    assert_eq!(task_142::KIND, TaskKind::Rust);
    let report = check_case("all_checks_pass_reportable");
    let m = &report.metrics;
    let checks: Vec<&str> = m["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for want in [
        "in-scope",
        "evidence-present",
        "non-duplicate",
        "reproducible",
    ] {
        assert!(
            checks.contains(&want),
            "pipeline must run {want}: {checks:?}"
        );
    }
    assert_eq!(m["state"].as_str().unwrap(), "Reportable");
}

/// V2: every single-check failure names its check and the fixture
/// stays Candidate — the attribution the rejection path depends on.
#[test]
fn each_check_blocks_with_name() {
    let report = check_case("each_check_blocks_with_name");
    let m = &report.metrics;
    assert_eq!(m["arms"].as_u64().unwrap(), 4);
    assert_eq!(m["blocked"].as_u64().unwrap(), 4);
    let joined = report.evidence.join("\n");
    for name in [
        "in-scope",
        "evidence-present",
        "non-duplicate",
        "reproducible",
    ] {
        assert!(
            joined.contains(name),
            "evidence must name the {name} arm:\n{joined}"
        );
    }
}

// --- adversarial ---

/// A1: the scope snapshot is unreachable — the check errors, and the
/// pipeline fails the finding closed. Never pass-on-error.
#[test]
fn validator_error_fails_closed() {
    let report = check_case("validator_error_fails_closed");
    let m = &report.metrics;
    assert!(m["fail_closed"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("fails closed"),
        "evidence must state the fail-closed rule:\n{joined}"
    );
}

/// A2: the operator's custom check is registered, blocks a bad finding,
/// lets a clean one through, and never weakens an old verdict.
#[test]
fn operator_check_extends_pipeline() {
    let report = check_case("operator_check_extends_pipeline");
    let m = &report.metrics;
    let checks: Vec<&str> = m["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        checks.contains(&"title-present"),
        "operator check must be registered: {checks:?}"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("never weakens"),
        "evidence must state the monotonicity result:\n{joined}"
    );
}
