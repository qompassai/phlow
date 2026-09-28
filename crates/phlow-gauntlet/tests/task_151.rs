// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-151 (open enums with Other(String)).
//!
//! Four validation cases against the scripted version-skewed peer
//! fixtures: an unknown `kind` variant deserializes to
//! `Kind::Other`, re-serializes byte-identical, and routes to the
//! default handler; 1,200 mixed variants parse with zero errors and
//! the catch-all bucket holding exactly the 1,000 unknowns; unknown
//! spellings never alias known variants; the adapted enum module
//! carries the maddada attribution.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_151;

fn check_case(case: &str) -> CaseReport {
    let report = task_151::run_case(case)
        .unwrap_or_else(|e| panic!("task-151 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-151 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: `kind: "ping_v9"` → `Other("ping_v9")`, byte-identical
/// re-serialization, default-handler routing.
#[test]
fn unknown_variant_round_trips() {
    assert_eq!(task_151::ID, "task-151");
    let report = check_case("unknown_variant_round_trips");
    let m = &report.metrics;
    assert!(
        m["round_trip_byte_identical"].as_bool().unwrap(),
        "the unknown variant must round-trip byte-identical"
    );
    assert!(m["dispatched_to_default"].as_bool().unwrap());
}

/// V2: 1,000 unknown + 200 known variants → zero errors, catch-all
/// bucket == 1,000.
#[test]
fn mixed_variants_no_errors() {
    let report = check_case("mixed_variants_no_errors");
    let m = &report.metrics;
    assert_eq!(m["total"].as_u64().unwrap(), 1200);
    assert_eq!(
        m["unknown_bucket"].as_u64().unwrap(),
        1000,
        "every unknown variant must land in the catch-all bucket"
    );
    assert_eq!(m["known_bucket"].as_u64().unwrap(), 200);
    assert_eq!(m["errors"].as_u64().unwrap(), 0);
}

/// Near-miss spellings never alias known variants; known spellings
/// stay intact.
#[test]
fn never_aliases_known() {
    let report = check_case("never_aliases_known");
    let m = &report.metrics;
    assert_eq!(m["near_misses"].as_u64().unwrap(), 8);
    assert_eq!(m["known_spellings"].as_u64().unwrap(), 7);
}

/// License gate: src/wire.rs and the task driver carry the maddada
/// attribution + source commit.
#[test]
fn license_header_present() {
    let report = check_case("license_header_present");
    assert_eq!(report.metrics["files_checked"].as_u64().unwrap(), 2);
}
