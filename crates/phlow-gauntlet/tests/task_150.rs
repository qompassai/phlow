//! Integration tests for task-150 (hostile scope-feed refusal).
//!
//! Four driver cases — 2 validation, 2 adversarial — against the
//! clearly labeled scripted double (ScriptedFeed + SignedFeed with toy
//! signature tags + fixture enrollment, deterministic, fast). A feed
//! that tries to authorize an out-of-bounds target is refused
//! structurally: the snapshot is rejected with a typed error, the feed
//! quarantined, the previous scope retained.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_150;

fn check_case(case: &str) -> CaseReport {
    let report = task_150::run_case(case)
        .unwrap_or_else(|e| panic!("task-150 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-150 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: *.evil.com against an example.com-only enrollment →
/// HostileTarget on both the self-reported poll and the snapshot; the
/// scope store stays at v5 and both refusals are quarantined.
#[test]
fn out_of_bounds_target_refused() {
    assert_eq!(task_150::ID, "task-150");
    let report = check_case("out_of_bounds_target_refused");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "HostileTarget");
    assert_eq!(
        m["scope_version"].as_u64().unwrap(),
        5,
        "the scope store must not change under attack"
    );
    assert_eq!(m["quarantined"].as_u64().unwrap(), 2);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("*.evil.com"),
        "evidence must name the hostile target:\n{joined}"
    );
}

/// V2: bad signature → BadSignature before any target is parsed. The
/// snapshot also carries a hostile target, so a HostileTarget refusal
/// here would prove the targets were inspected — it must not happen.
#[test]
fn bad_signature_rejected_before_parse() {
    let report = check_case("bad_signature_rejected_before_parse");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "BadSignature");
    assert_eq!(m["scope_version"].as_u64().unwrap(), 5);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("targets never parsed"),
        "evidence must prove parse order:\n{joined}"
    );
}

// --- adversarial ---

/// A1: 10.0.0.0/24 → 10.0.0.0/8 widening → HostileTarget naming the
/// widening; the enrolled /24 stays the filed scope.
#[test]
fn cidr_widening_refused() {
    let report = check_case("cidr_widening_refused");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "HostileTarget");
    assert_eq!(m["scope_version"].as_u64().unwrap(), 5);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("widens enrolled target 10.0.0.0/24"),
        "evidence must name the widening:\n{joined}"
    );
}

/// A2: a correctly signed v3 replayed after v5 → StaleVersion; the
/// store keeps v5.
#[test]
fn replayed_version_rejected() {
    let report = check_case("replayed_version_rejected");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "StaleVersion");
    assert_eq!(m["scope_version"].as_u64().unwrap(), 5);
    assert_eq!(m["quarantined"].as_u64().unwrap(), 1);
}
