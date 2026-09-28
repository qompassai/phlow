// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-160 (subscribe-on-reconnect).
//!
//! Two driver cases against the client-held [`SubscriptionSet`] +
//! [`ScriptedLink`] doubles (deterministic, fast): V1 asserts the daemon
//! observes subscribe(a), subscribe(b) exactly once per connection
//! across a drop; V2 asserts subscribe(c) during the outage lands in
//! the post-reconnect set with no duplicates.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_160;

fn check_case(case: &str) -> CaseReport {
    let report = task_160::run_case(case)
        .unwrap_or_else(|e| panic!("task-160 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-160 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: {a, b} subscribed, drop, reconnect → daemon-side counts are
/// exactly a:2, b:2 (once per connection), link released exactly once
/// per connect.
#[test]
fn resubscribe_exactly_once() {
    assert_eq!(task_160::ID, "task-160");
    let report = check_case("resubscribe_exactly_once");
    let m = &report.metrics;
    let counts = m["subscribe_counts"].as_object().unwrap();
    assert_eq!(counts["a"].as_u64().unwrap(), 2);
    assert_eq!(counts["b"].as_u64().unwrap(), 2);
    assert_eq!(counts.len(), 2, "no other topic may be subscribed");
    assert_eq!(m["connects"].as_u64().unwrap(), 2);
    assert_eq!(m["closes"].as_u64().unwrap(), 2);
    assert_eq!(m["open_handles"].as_u64().unwrap(), 0);
}

/// V2: subscribe(c) during the outage → post-reconnect batch is exactly
/// [sub:a, sub:b, sub:c], daemon-side counts a:2, b:2, c:1, no dupes.
#[test]
fn subscribe_during_outage() {
    let report = check_case("subscribe_during_outage");
    let m = &report.metrics;
    let batch: Vec<&str> = m["post_reconnect_batch"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(batch, vec!["sub:a", "sub:b", "sub:c"]);
    let counts = m["subscribe_counts"].as_object().unwrap();
    assert_eq!(counts["a"].as_u64().unwrap(), 2);
    assert_eq!(counts["b"].as_u64().unwrap(), 2);
    assert_eq!(counts["c"].as_u64().unwrap(), 1);
    assert_eq!(counts.len(), 3);
}
