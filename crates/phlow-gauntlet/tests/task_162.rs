// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-162 (flapping daemon backoff).
//!
//! Two adversarial driver cases against the deterministic
//! [`ReconnectEngine`] + [`ScriptedLink`] doubles: A1 hammers 1,000
//! accept-then-drop flaps and asserts the attempt cap, the ladder floor,
//! and a clean handle/fd census; A2 syncs abrupt drops to the
//! ladder-reset boundary and asserts the consecutive-failure counter
//! survives (no collapse to the minimum interval).

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_162;

fn check_case(case: &str) -> CaseReport {
    let report = task_162::run_case(case)
        .unwrap_or_else(|e| panic!("task-162 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-162 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- adversarial ---

/// A1: 1,000 flaps → attempts capped at MAX_RECONNECT_ATTEMPTS, minimum
/// inter-attempt interval at or above the ladder floor, fd count stable.
#[test]
fn thousand_flaps_capped() {
    assert_eq!(task_162::ID, "task-162");
    let report = check_case("thousand_flaps_capped");
    let m = &report.metrics;
    assert_eq!(m["flaps"].as_u64().unwrap(), 1000);
    assert_eq!(
        m["attempts"].as_u64().unwrap(),
        m["max_reconnect_attempts"].as_u64().unwrap(),
        "the flap storm must not produce more attempts than the cap"
    );
    assert!(
        m["min_interval_ms"].as_u64().unwrap() >= 100,
        "no inter-attempt interval may dip below the ladder floor"
    );
    assert_eq!(
        m["fd_before"].as_u64().unwrap(),
        m["fd_after"].as_u64().unwrap(),
        "fd count must be stable across the flap storm"
    );
}

/// A2: 7 boundary-synced abrupt drops → the counter reaches 7 (it
/// survives every reset point) and the next delay is the top rung, not
/// the floor; a genuine healthy + orderly close still earns the reset.
#[test]
fn synced_flap_no_collapse() {
    let report = check_case("synced_flap_no_collapse");
    let m = &report.metrics;
    assert_eq!(m["counter_after_synced"].as_u64().unwrap(), 7);
    assert_eq!(
        m["delay_after_synced_ms"].as_u64().unwrap(),
        m["ladder_top_ms"].as_u64().unwrap(),
        "the ladder must stay escalated, not collapse to the floor"
    );
    assert!(m["delay_after_synced_ms"].as_u64().unwrap() > m["ladder_floor_ms"].as_u64().unwrap());
    assert!(m["reset_earned"].as_bool().unwrap());
}
