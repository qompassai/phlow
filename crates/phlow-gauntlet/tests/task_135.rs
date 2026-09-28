//! Integration tests for task-135 (approval expiry).
//!
//! Four driver cases — 2 validation, 2 adversarial — on the
//! [`ManualClock`]: launch at T=expiry-1 allowed; launch at the
//! exact expiry second and after refused with `GateError::Expired`;
//! an in-flight run may finish past expiry while a new launch is
//! refused (ledger distinguishes the two); a backdated approval is
//! rejected at issuance and never stored.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_135;

fn check_case(case: &str) -> CaseReport {
    let report = task_135::run_case(case)
        .unwrap_or_else(|e| panic!("task-135 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-135 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: launch one second before expiry is allowed, bound to the
/// approval's nonce.
#[test]
fn launch_before_expiry() {
    assert_eq!(task_135::ID, "task-135");
    assert_eq!(task_135::APPROVAL_TTL_SECS, 3600);
    let report = check_case("launch_before_expiry");
    let m = &report.metrics;
    assert_eq!(
        m["launched_at"].as_u64(),
        Some(task_135::CLOCK_START + 3599)
    );
    assert_eq!(
        m["expires_at"].as_u64(),
        Some(task_135::CLOCK_START + task_135::APPROVAL_TTL_SECS)
    );
}

/// V2: the boundary is exact at the expiry second — T=expiry and
/// T=expiry+1 both refused with `GateError::Expired`.
#[test]
fn launch_after_expiry() {
    let report = check_case("launch_after_expiry");
    assert_eq!(report.metrics["refused_at_boundary"].as_bool(), Some(true));
}

// --- adversarial ---

/// A1: a run launched before expiry finishes past expiry (gate is
/// launch-only), but a new launch past expiry is refused; the ledger
/// holds exactly one run.
#[test]
fn inflight_finishes_new_blocked() {
    let report = check_case("inflight_finishes_new_blocked");
    let m = &report.metrics;
    assert_eq!(m["inflight_finished"].as_bool(), Some(true));
    assert_eq!(m["new_launch_refused"].as_bool(), Some(true));
    assert_eq!(m["ledger_runs"].as_u64(), Some(1));
}

/// A2: backdated approval rejected at issuance with
/// `IssuanceError::NotYetGranted`; the store stays empty.
#[test]
fn backdated_rejected_at_issuance() {
    let report = check_case("backdated_rejected_at_issuance");
    assert_eq!(report.metrics["store_size"].as_u64(), Some(0));
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("NotYetGranted") && joined.contains("NeverLive"),
        "evidence must show both issuance refusals:\n{joined}"
    );
}
