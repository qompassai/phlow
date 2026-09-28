//! Integration tests for task-148 (triager feedback ingestion).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! clearly labeled scripted double (FakePlatform events +
//! driver-local FeedbackIngester, deterministic, fast). NeedsMoreInfo
//! routes back to validation, never to recon; new evidence re-enters
//! the full pipeline and returns to Reportable; terminal states reject
//! feedback; unknown finding ids are a typed error with nothing
//! created implicitly.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_148;

fn check_case(case: &str) -> CaseReport {
    let report = task_148::run_case(case)
        .unwrap_or_else(|e| panic!("task-148 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-148 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: NeedsMoreInfo → Triage becomes NeedsMoreInfo, queued for
/// validation; the recon queue must stay empty (the cycle does not
/// restart).
#[test]
fn needs_more_info_routes_to_validation() {
    assert_eq!(task_148::ID, "task-148");
    let report = check_case("needs_more_info_routes_to_validation");
    let m = &report.metrics;
    assert_eq!(m["state"].as_str().unwrap(), "NeedsMoreInfo");
    assert_eq!(m["validation_queue"].as_array().unwrap().len(), 1);
    assert_eq!(
        m["recon_queue"].as_array().unwrap().len(),
        0,
        "feedback must never restart recon"
    );
}

/// V2: new evidence re-enters the full pipeline; all default checks
/// pass and the finding is Reportable again.
#[test]
fn new_evidence_revalidates_to_reportable() {
    let report = check_case("new_evidence_revalidates_to_reportable");
    let m = &report.metrics;
    assert_eq!(m["state"].as_str().unwrap(), "Reportable");
    let checks: Vec<&str> = m["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for name in ["in-scope", "evidence-present", "non-duplicate"] {
        assert!(
            checks.contains(&name),
            "the full pipeline must run, missing check {name}: {checks:?}"
        );
    }
}

// --- adversarial ---

/// A1: NeedsMoreInfo for an Accepted finding → IllegalTransition;
/// terminal states are terminal.
#[test]
fn terminal_finding_rejects_feedback() {
    let report = check_case("terminal_finding_rejects_feedback");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "IllegalTransition");
    assert_eq!(m["state"].as_str().unwrap(), "Accepted");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("from: Accepted, to: NeedsMoreInfo"),
        "evidence must name the refused transition:\n{joined}"
    );
}

/// A2: feedback for an unknown finding id → UnknownFinding; the
/// registry is untouched and nothing is queued.
#[test]
fn unknown_finding_id_typed_error() {
    let report = check_case("unknown_finding_id_typed_error");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "UnknownFinding");
    assert_eq!(m["registry_size"].as_u64().unwrap(), 1);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("UnknownFinding"),
        "evidence must name the typed refusal:\n{joined}"
    );
}
