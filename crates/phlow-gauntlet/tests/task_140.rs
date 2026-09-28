//! Integration tests for task-140 (rate limits and testing windows).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! scaffold's `Scheduler` with a scripted shared-cell clock (MOCK) and
//! scripted 429s from `FakePlatform`. Launches proceed inside the
//! 02:00–04:00 UTC window; a tick at 05:00 holds with the queue intact;
//! the window closing with runs in flight finishes them but launches
//! nothing new; 429s drive exponential backoff (1,2,4,… capped at 300s)
//! with the attempt rate bounded by limit × elapsed + burst.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_140;

fn check_case(case: &str) -> CaseReport {
    let report = task_140::run_case(case)
        .unwrap_or_else(|e| panic!("task-140 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-140 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: tick at 03:00 — launches proceed, all 5 targets launched.
#[test]
fn window_open_launches() {
    assert_eq!(task_140::ID, "task-140");
    let report = check_case("window_open_launches");
    let m = &report.metrics;
    assert_eq!(m["launched"].as_u64().unwrap(), 5);
    assert!(m["queue_empty"].as_bool().unwrap());
}

/// V2: tick at 05:00 — Hold, queue intact, zero launches.
#[test]
fn window_closed_holds() {
    let report = check_case("window_closed_holds");
    let m = &report.metrics;
    assert_eq!(
        m["launches"].as_u64().unwrap(),
        0,
        "zero launches outside the window"
    );
    assert_eq!(
        m["queue_len"].as_u64().unwrap(),
        5,
        "queue must stay intact"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("testing window closed"),
        "evidence must name the hold reason:\n{joined}"
    );
}

// --- adversarial ---

/// A1: window closes with 2 in flight — they finish, nothing new
/// launches after 04:00.
#[test]
fn window_close_keeps_inflight() {
    let report = check_case("window_close_keeps_inflight");
    let m = &report.metrics;
    assert_eq!(m["launched_before_close"].as_u64().unwrap(), 2);
    assert_eq!(
        m["launches_after_close"].as_u64().unwrap(),
        0,
        "zero new launches after the window closes"
    );
    assert_eq!(m["queue_len"].as_u64().unwrap(), 2);
    assert_eq!(m["in_flight"].as_u64().unwrap(), 0);
}

/// A2: 3 scripted 429s — exponential gaps 1,2,4; attempts within
/// limit × elapsed + burst; backoff capped at 300s.
#[test]
fn rate_limit_429_backoff() {
    let report = check_case("rate_limit_429_backoff");
    let m = &report.metrics;
    assert_eq!(m["attempts"].as_u64().unwrap(), 4, "3 refused + 1 accepted");
    let gaps: Vec<u64> = m["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(gaps, vec![1, 2, 4], "backoff must be exponential");
    assert!(
        m["attempts"].as_u64().unwrap() <= m["bound"].as_u64().unwrap(),
        "no retry storm: attempts within limit*elapsed+burst"
    );
    let cap: Vec<u64> = m["backoff_cap_schedule"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(
        cap,
        vec![1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300],
        "backoff must cap at 300s"
    );
}
