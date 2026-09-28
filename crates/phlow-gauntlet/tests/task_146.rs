//! Integration tests for task-146 (operator-approval boundary).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! clearly labeled scripted double (SubmissionGate + fixture approvals
//! + ManualClock, deterministic, fast):
//!
//! - valid approval with a matching payload hash submits;
//! - all-green finding with no approval is refused with `NoApproval`;
//! - approval binding different bytes is refused with `HashMismatch`
//!   (nonce unspent);
//! - replayed nonce is refused with `ReplayNonce`.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_146;

fn check_case(case: &str) -> CaseReport {
    let report = task_146::run_case(case)
        .unwrap_or_else(|e| panic!("task-146 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-146 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: valid approval + matching hash → submitted, nonce spent once.
#[test]
fn valid_approval_and_hash_submits() {
    assert_eq!(task_146::ID, "task-146");
    let report = check_case("valid_approval_and_hash_submits");
    let m = &report.metrics;
    assert_eq!(m["spent_nonces"].as_u64().unwrap(), 1);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("approved_hash="),
        "evidence must show the bound hash:\n{joined}"
    );
}

/// V2: all-green finding, no approval → NoApproval. Greenness is not
/// authorization.
#[test]
fn no_approval_refused() {
    let report = check_case("no_approval_refused");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "NoApproval");
    assert_eq!(m["spent_nonces"].as_u64().unwrap(), 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("NoApproval"),
        "evidence must name the typed refusal:\n{joined}"
    );
}

// --- adversarial ---

/// A1: approval binds v1, payload is v2 → HashMismatch, and the failed
/// attack must not consume the nonce.
#[test]
fn hash_mismatch_refused() {
    let report = check_case("hash_mismatch_refused");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "HashMismatch");
    assert_eq!(
        m["spent_nonces"].as_u64().unwrap(),
        0,
        "a refused submission must not spend the nonce"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("approved(v1)=") && joined.contains("submitted(v2)="),
        "evidence must show both hashes:\n{joined}"
    );
}

/// A2: replaying an approval nonce → ReplayNonce; the spent set holds
/// exactly the one legitimate submission.
#[test]
fn replay_nonce_refused() {
    let report = check_case("replay_nonce_refused");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "ReplayNonce");
    assert_eq!(m["spent_nonces"].as_u64().unwrap(), 1);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("ReplayNonce"),
        "evidence must name the typed refusal:\n{joined}"
    );
}
