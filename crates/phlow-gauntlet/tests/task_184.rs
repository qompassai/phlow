// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-184 (bridge cross-talk isolation).
//!
//! Three adversarial cases with two concurrent bridges: T1's page
//! exfiltrates a marker via evaluate -> the marker never appears in
//! T2's result stream; T1 fed T2's target id (raw and namespaced) ->
//! UnknownTarget in T1's namespace; the per-bridge target-id
//! namespaces are disjoint by construction.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_184;

fn check_case(case: &str) -> CaseReport {
    let report = task_184::run_case(case)
        .unwrap_or_else(|e| panic!("task-184 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-184 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: marker in T1's stream, absent from T2's (scanned, 3 evaluates).
#[test]
fn no_result_crosstalk() {
    assert_eq!(task_184::ID, "task-184");
    let report = check_case("no_result_crosstalk");
    assert!(!report.metrics["crosstalk"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("marker absent"),
        "evidence must show the scan result:\n{joined}"
    );
    assert!(
        !joined.contains("CROSSTALK"),
        "no cross-talk may be reported:\n{joined}"
    );
}

/// A2: three foreign ids (raw colliding id, T2's namespaced id, a
/// bogus T2 id) -> UnknownTarget every time; T1's own id still works.
#[test]
fn cross_target_id_unknown() {
    let report = check_case("cross_target_id_unknown");
    assert_eq!(report.metrics["foreign_ids_refused"].as_u64().unwrap(), 3);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("UnknownTarget"),
        "evidence must name the typed refusal:\n{joined}"
    );
    assert!(
        joined.contains("collides with T1's raw id by design"),
        "evidence must note the fixture sharpness:\n{joined}"
    );
}

/// Namespaces disjoint by construction: task-prefixed ids, no overlap.
#[test]
fn namespaces_disjoint_by_construction() {
    let report = check_case("namespaces_disjoint_by_construction");
    assert_eq!(report.metrics["overlap"].as_u64().unwrap(), 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("disjoint"),
        "evidence must state disjointness:\n{joined}"
    );
}
