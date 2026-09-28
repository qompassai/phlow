//! Integration tests for task-147 (post-submission state tracking).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! clearly labeled scripted double (FakePlatform triage events +
//! driver-local TriageTracker, deterministic, fast). Legal sequences
//! are tracked exactly; unknown platform states are recorded, never
//! mapped; illegal transitions quarantine the event with the state
//! untouched.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_147;

fn check_case(case: &str) -> CaseReport {
    let report = task_147::run_case(case)
        .unwrap_or_else(|e| panic!("task-147 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-147 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: Submitted → Triage → Accepted tracked exactly; the ledger holds
/// both transitions in order and nothing is quarantined.
#[test]
fn submitted_triage_accepted() {
    assert_eq!(task_147::ID, "task-147");
    let report = check_case("submitted_triage_accepted");
    let m = &report.metrics;
    assert_eq!(m["final_state"].as_str().unwrap(), "Accepted");
    assert_eq!(m["events"].as_u64().unwrap(), 2);
    assert_eq!(m["quarantined"].as_u64().unwrap(), 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("Submitted -> Triage") && joined.contains("Triage -> Accepted"),
        "evidence must show the tracked sequence:\n{joined}"
    );
}

/// V2: DuplicateOf(orig-123) → Duplicate, linked to the original
/// report id.
#[test]
fn duplicate_links_original() {
    let report = check_case("duplicate_links_original");
    let m = &report.metrics;
    assert_eq!(m["final_state"].as_str().unwrap(), "Duplicate");
    let links = m["duplicate_of"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0][1].as_str().unwrap(), "orig-123");
}

// --- adversarial ---

/// A1: an unknown platform state is recorded verbatim; the finding
/// state never moves on input the tracker does not understand.
#[test]
fn unknown_state_never_mapped() {
    let report = check_case("unknown_state_never_mapped");
    let m = &report.metrics;
    assert_eq!(m["state"].as_str().unwrap(), "Triage");
    assert_eq!(m["quarantined"].as_u64().unwrap(), 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("Unknown(triaged_by_contractor)"),
        "evidence must record the unknown state verbatim:\n{joined}"
    );
}

/// A2: Accepted for a finding still in Submitted → IllegalTransition;
/// the event is quarantined and the state is untouched.
#[test]
fn skipped_state_rejected() {
    let report = check_case("skipped_state_rejected");
    let m = &report.metrics;
    assert_eq!(m["rejection"].as_str().unwrap(), "IllegalTransition");
    assert_eq!(m["state"].as_str().unwrap(), "Submitted");
    assert_eq!(m["quarantined"].as_u64().unwrap(), 1);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("from: Submitted, to: Accepted"),
        "evidence must name the illegal transition:\n{joined}"
    );
}
