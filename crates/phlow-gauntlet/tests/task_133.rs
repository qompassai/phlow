//! Integration tests for task-133 (scope revocation mid-cycle).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! scripted feed (v1, v2 drops b, v3 re-adds b): a queued revoked
//! target is dropped and never launched; a running revoked target's
//! run is cancelled with the `ScopeRevoked` reason; an unknown
//! revocation is a typed no-op; a re-added target is queueable again
//! under the latest snapshot.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_133;

fn check_case(case: &str) -> CaseReport {
    let report = task_133::run_case(case)
        .unwrap_or_else(|e| panic!("task-133 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-133 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: b revoked while queued → dropped from the queue, zero ledger
/// runs for b after v2 (never launched).
#[test]
fn revoked_queued_target_dropped() {
    assert_eq!(task_133::ID, "task-133");
    assert_eq!(task_133::SCOPE_REVOKED_REASON, "ScopeRevoked");
    let report = check_case("revoked_queued_target_dropped");
    let m = &report.metrics;
    assert_eq!(m["b_revoked_from_queue"].as_bool(), Some(true));
    assert_eq!(m["b_runs"].as_u64(), Some(0), "b must never launch");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("Applied"),
        "evidence must show the revocation outcome:\n{joined}"
    );
}

/// V2: c revoked while Running → Cancelled with the `ScopeRevoked`
/// reason recorded.
#[test]
fn revoked_running_run_cancelled() {
    let report = check_case("revoked_running_run_cancelled");
    let m = &report.metrics;
    assert_eq!(m["c_run_state"].as_str(), Some("Some(Cancelled)"));
    assert_eq!(
        m["cancel_reason"].as_str(),
        Some(task_133::SCOPE_REVOKED_REASON)
    );
}

// --- adversarial ---

/// A1: revocation naming an unknown target → typed `UnknownTarget`
/// no-op; queue and ledger untouched.
#[test]
fn unknown_revocation_noop() {
    let report = check_case("unknown_revocation_noop");
    let m = &report.metrics;
    assert_eq!(m["outcome"].as_str(), Some("UnknownTarget"));
    assert_eq!(m["queue_untouched"].as_bool(), Some(true));
    assert_eq!(m["ledger_untouched"].as_bool(), Some(true));
}

/// A2: b re-added in v3 → refused under v2 (typed `NotInScope`),
/// queueable under v3; latest snapshot wins, no permanent ban.
#[test]
fn readded_target_queueable() {
    let report = check_case("readded_target_queueable");
    let m = &report.metrics;
    assert_eq!(m["refused_under_v2"].as_bool(), Some(true));
    assert_eq!(m["queueable_under_v3"].as_bool(), Some(true));
}
