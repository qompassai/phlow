// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-182 (bridge navigation allowlist).
//!
//! Four adversarial cases against the scripted port: file:// and
//! data:/javascript: URLs are denied before the port is touched;
//! off-allowlist hosts are denied with the dot-boundary holding;
//! a page-side window.open on a single-target task is refused and
//! closed; a declared multi-target task adopts up to its bound and
//! refuses beyond it with TargetLimitExceeded.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_182;

fn check_case(case: &str) -> CaseReport {
    let report = task_182::run_case(case)
        .unwrap_or_else(|e| panic!("task-182 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-182 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1a: file:///etc/passwd (plus data:/javascript:) -> NavigationDenied,
/// port never touched.
#[test]
fn file_url_denied() {
    assert_eq!(task_182::ID, "task-182");
    let report = check_case("file_url_denied");
    let m = &report.metrics;
    assert_eq!(m["denied"].as_u64().unwrap(), 3);
    assert_eq!(m["port_navigations"].as_u64().unwrap(), 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("NavigationDenied"),
        "evidence must name the typed denial:\n{joined}"
    );
}

/// A1b: evil.com, example.com.evil.com, notexample.com denied; a real
/// subdomain allowed and observed exactly once.
#[test]
fn off_allowlist_host_denied() {
    let report = check_case("off_allowlist_host_denied");
    let m = &report.metrics;
    assert_eq!(m["denied"].as_u64().unwrap(), 3);
    assert_eq!(m["allowed"].as_u64().unwrap(), 1);
}

/// A2a: window.open on a single-target task -> TargetRefused, closed,
/// live count stays 1.
#[test]
fn window_open_refused_single_target() {
    let report = check_case("window_open_refused_single_target");
    let m = &report.metrics;
    assert_eq!(m["refusals"].as_u64().unwrap(), 1);
    assert_eq!(m["live_targets"].as_u64().unwrap(), 1);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("TargetRefused"),
        "evidence must name the typed refusal:\n{joined}"
    );
}

/// A2b: declared multi-target bound 2 — first popup adopted
/// (namespaced), second -> TargetLimitExceeded, count never exceeds 2.
#[test]
fn multi_target_declared_bound() {
    let report = check_case("multi_target_declared_bound");
    let m = &report.metrics;
    assert_eq!(m["bound"].as_u64().unwrap(), 2);
    assert_eq!(m["live_targets"].as_u64().unwrap(), 2);
    assert_eq!(m["targets_closed"].as_u64().unwrap(), 2);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("TargetLimitExceeded"),
        "evidence must name the bound refusal:\n{joined}"
    );
}
