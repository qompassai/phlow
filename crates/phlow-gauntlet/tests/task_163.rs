// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-163 (hostile peer on the daemon socket).
//!
//! Two adversarial driver cases against the real loopback-TCP
//! [`DaemonFixture`]: A1 asserts unauthenticated privileged frames get
//! `AuthError` + drop + log with zero privileged effects (and that a
//! credentialed frame passes, proving the gate is auth, not framing);
//! A2 asserts a replayed valid snapshot is rejected as `StaleSnapshot`
//! with daemon state unchanged, and that auth is checked before
//! staleness.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_163;

fn check_case(case: &str) -> CaseReport {
    let report = task_163::run_case(case)
        .unwrap_or_else(|e| panic!("task-163 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-163 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- adversarial ---

/// A1: cred-less `daemon-shutdown` and wrong-cred `exec` → `AuthError`;
/// zero privileged effects; credentialed `ping` still works.
#[test]
fn unauthenticated_privileged_refused() {
    assert_eq!(task_163::ID, "task-163");
    let report = check_case("unauthenticated_privileged_refused");
    let m = &report.metrics;
    assert!(
        m["auth_errors"].as_u64().unwrap() >= 2,
        "both hostile probes must be logged as AuthError"
    );
    assert_eq!(
        m["privileged_effects"].as_u64().unwrap(),
        0,
        "zero privileged effects from unauthenticated peers"
    );
    assert!(
        m["positive_control_ok"].as_bool().unwrap(),
        "a credentialed frame must pass: the gate is auth, not framing"
    );
}

/// A2: replayed valid snapshot (good cred, stale epoch) →
/// `StaleSnapshot` with state unchanged; cred-less replay → `AuthError`
/// (auth checked before staleness).
#[test]
fn replayed_snapshot_stale() {
    let report = check_case("replayed_snapshot_stale");
    let m = &report.metrics;
    assert!(
        m["stale_rejections"].as_u64().unwrap() >= 2,
        "both stale replays must be rejected and logged"
    );
    assert!(
        m["state_unchanged"].as_bool().unwrap(),
        "the replay must not mutate daemon state"
    );
    assert!(
        m["auth_before_staleness"].as_bool().unwrap(),
        "a credential-less replay must fail auth, not staleness"
    );
}
