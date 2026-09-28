//! Integration tests for task-136 (bounded concurrency queueing).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the clearly
//! labeled scripted double (ManualClock + scripted probe durations).
//! K=3 over 10 targets: peak exactly 3, all finish. K=1: strictly
//! serial. A crashed worker is failed by the heartbeat watchdog, its
//! slot released within the bound, and the queue keeps draining.
//! K=0 is refused at construction.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_136;

fn check_case(case: &str) -> CaseReport {
    let report = task_136::run_case(case)
        .unwrap_or_else(|e| panic!("task-136 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-136 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: peak concurrent == 3, all 10 targets finish.
#[test]
fn peak_bound_k3() {
    assert_eq!(task_136::ID, "task-136");
    let report = check_case("peak_bound_k3");
    let m = &report.metrics;
    assert_eq!(m["peak"].as_u64().unwrap(), 3, "peak must be exactly K=3");
    assert_eq!(m["finished"].as_u64().unwrap(), 10, "all 10 must finish");
    assert_eq!(m["failed"].as_u64().unwrap(), 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("peak concurrent = 3"),
        "evidence must show the peak:\n{joined}"
    );
}

/// V2: K=1 is strictly serial — peak 1, everything still completes.
#[test]
fn serial_at_k1() {
    let report = check_case("serial_at_k1");
    let m = &report.metrics;
    assert_eq!(m["peak"].as_u64().unwrap(), 1, "K=1 must be serial");
    assert_eq!(m["finished"].as_u64().unwrap(), 6);
}

// --- adversarial ---

/// A1: the crashed worker's run is Failed by the watchdog, the slot is
/// released within heartbeat_timeout + 1 ticks, and the queue drains.
#[test]
fn crash_releases_slot() {
    let report = check_case("crash_releases_slot");
    let m = &report.metrics;
    assert_eq!(m["failed"].as_u64().unwrap(), 1);
    assert_eq!(m["finished"].as_u64().unwrap(), 3);
    let failed_at = m["failed_at_tick"].as_u64().unwrap();
    let launched_at = m["crash_launch_tick"].as_u64().unwrap();
    assert_eq!(
        failed_at - launched_at,
        task_136::HEARTBEAT_TIMEOUT_TICKS + 1,
        "slot must release exactly timeout+1 ticks after the last heartbeat"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("heartbeat-timeout"),
        "evidence must name the watchdog reason:\n{joined}"
    );
}

/// A2: K=0 is refused at construction — fail closed.
#[test]
fn k0_refused() {
    let report = check_case("k0_refused");
    let m = &report.metrics;
    assert!(m["refused"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("max_concurrent"),
        "evidence must show the typed refusal:\n{joined}"
    );
}
