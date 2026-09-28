// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-170 (pairing code issuance).
//!
//! Two validation cases against the in-memory [`Daemon`] with a
//! [`ManualClock`]: the issued code has the exact
//! `phlow-ec1:<base64url>` shape with `issued_at`, `ttl_secs == 900`
//! and the label in its payload, and the store holds the secret's
//! hash and never the secret; presenting within the TTL pairs
//! exactly once.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_170;

fn check_case(case: &str) -> CaseReport {
    let report = task_170::run_case(case)
        .unwrap_or_else(|e| panic!("task-170 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-170 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: exact code shape; payload decodes with `issued_at`,
/// `ttl_secs == 900`, and the label; the store scan finds the
/// secret's hash and not the secret.
#[test]
fn issue_shape_and_hash_only() {
    assert_eq!(task_170::ID, "task-170");
    let report = check_case("issue_shape_and_hash_only");
    let m = &report.metrics;
    assert!(m["prefix_ok"].as_bool().unwrap());
    assert!(m["body_base64url"].as_bool().unwrap());
    assert_eq!(m["ttl_secs"].as_u64().unwrap(), 900);
    assert_eq!(m["label"].as_str().unwrap(), "pixel-9");
    assert_eq!(m["issued_at"].as_u64().unwrap(), task_170::CLOCK_START);
    assert!(!m["secret_present"].as_bool().unwrap());
    assert!(m["hash_present"].as_bool().unwrap());
}

/// V2: presenting within the TTL pairs once and consumes the code.
#[test]
fn present_pairs_once() {
    let report = check_case("present_pairs_once");
    let m = &report.metrics;
    assert!(m["paired"].as_bool().unwrap());
    assert_eq!(m["device_count"].as_u64().unwrap(), 1);
    assert!(m["consumed_on_reuse"].as_bool().unwrap());
}
