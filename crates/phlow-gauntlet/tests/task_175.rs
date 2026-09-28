// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Integration tests for task-175 (forged code rejection).
//!
//! Two adversarial cases: a deterministic 200-forgery bit-flip
//! corpus (PCG32, fixed seed) is refused as `Malformed` or
//! `Authenticity` — never `Mismatch` — with the comparison counter
//! unmoved; a code minted by a foreign daemon instance is refused as
//! `Authenticity`, a `ghostex-ec1:`-prefixed code as `Malformed`, and
//! the home daemon's genuine code still pairs afterwards.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_175;

fn check_case(case: &str) -> CaseReport {
    let report = task_175::run_case(case)
        .unwrap_or_else(|e| panic!("task-175 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-175 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: 200 bit-flipped forgeries refused before the secret compare.
#[test]
fn fuzz_corpus_rejected() {
    assert_eq!(task_175::ID, "task-175");
    let report = check_case("fuzz_corpus_rejected");
    let m = &report.metrics;
    assert_eq!(m["corpus"].as_u64().unwrap(), 200);
    let refused = m["malformed"].as_u64().unwrap() + m["authenticity"].as_u64().unwrap();
    assert_eq!(
        refused, 200,
        "every forgery must die as Malformed or Authenticity"
    );
    assert_eq!(
        m["comparisons_after"].as_u64().unwrap(),
        m["comparisons_before"].as_u64().unwrap(),
        "no forgery may reach the secret comparison"
    );
    assert_eq!(m["pairings"].as_u64().unwrap(), 0);
}

/// A2: foreign instance key and wrong prefix refused; genuine code
/// still pairs.
#[test]
fn foreign_instance_and_prefix() {
    let report = check_case("foreign_instance_and_prefix");
    let m = &report.metrics;
    assert!(m["foreign_refused_as_authenticity"].as_bool().unwrap());
    assert!(m["ghostex_prefix_refused_as_malformed"].as_bool().unwrap());
    assert!(m["comparisons_unchanged"].as_bool().unwrap());
    assert!(m["genuine_still_pairs"].as_bool().unwrap());
}
