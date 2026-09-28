// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-159 (reconnect ladder).
//!
//! Two driver cases against the deterministic [`ReconnectEngine`] driven
//! by the scripted [`ManualClock`] + [`ScriptedLink`] doubles
//! (deterministic, fast): V1 asserts the three drops land on the ladder
//! intervals and the fourth attempt succeeds with a ladder reset; V2
//! asserts the daemon-down run parks in `BackoffExhausted` with zero
//! wakeups over 60 s of parked clock time.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_159;

fn check_case(case: &str) -> CaseReport {
    let report = task_159::run_case(case)
        .unwrap_or_else(|e| panic!("task-159 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-159 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: 3 drops → attempts at exactly T0, T0+100, T0+600, T0+2600 (the
/// ladder [100, 500, 2000] cumulative); the 4th attempt succeeds and a
/// healthy + orderly close resets the ladder to the floor rung.
#[test]
fn reconnect_ladder_three_drops() {
    assert_eq!(task_159::ID, "task-159");
    let report = check_case("reconnect_ladder_three_drops");
    let m = &report.metrics;
    let times = m["attempt_times_ms"].as_array().unwrap();
    assert_eq!(times.len(), 4, "want 4 attempts, got {times:?}");
    let t0 = times[0].as_u64().unwrap();
    let gaps: Vec<u64> = times
        .windows(2)
        .map(|w| w[1].as_u64().unwrap() - w[0].as_u64().unwrap())
        .collect();
    assert_eq!(
        gaps,
        vec![100, 500, 2000],
        "attempt gaps must be the ladder rungs"
    );
    assert!(
        m["reset_observed"].as_bool().unwrap(),
        "the ladder must reset after a healthy, orderly close"
    );
    assert!(!(m["parked"].as_bool().unwrap()));
    let _ = t0;
}

/// V2: daemon down for the whole test → exactly MAX_RECONNECT_ATTEMPTS
/// attempts, parked in BackoffExhausted, zero wakeups over 60 s of
/// parked ManualClock time (no busy loop).
#[test]
fn backoff_exhausted_parks() {
    let report = check_case("backoff_exhausted_parks");
    let m = &report.metrics;
    assert_eq!(
        m["attempts"].as_u64().unwrap(),
        m["max_reconnect_attempts"].as_u64().unwrap(),
        "attempts must stop exactly at the cap"
    );
    assert_eq!(m["parked"].as_str().unwrap(), "Some(BackoffExhausted)");
    assert_eq!(
        m["parked_window_wakeups"].as_u64().unwrap(),
        0,
        "a parked engine must perform zero wakeups (deadline-driven, not spinning)"
    );
}
