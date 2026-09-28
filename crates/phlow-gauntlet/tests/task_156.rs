// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-156 (variant-confusion refusal).
//!
//! Four adversarial cases: seven hostile kind spellings
//! (fake-privileged names, case variants, whitespace smuggling, an
//! embedded NUL, Unicode confusables) all land in `Other(..)` and
//! route to the default handler; the hostile payloads are captured
//! byte-identical (never normalized) while the exact `"ping"`
//! spelling still reaches the Ping arm; the privileged `Subscribe`
//! arm fires zero times from hostile variants while the exact
//! `"subscribe"` spelling reaches it; the adapted wire module carries
//! the maddada attribution.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_156;

fn check_case(case: &str) -> CaseReport {
    let report = task_156::run_case(case)
        .unwrap_or_else(|e| panic!("task-156 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-156 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: all seven hostile spellings → Other(..) → default handler.
#[test]
fn confusable_variants_defaulted() {
    let report = check_case("confusable_variants_defaulted");
    let m = &report.metrics;
    assert_eq!(m["hostile_spellings"].as_u64().unwrap(), 7);
    assert_eq!(m["defaulted"].as_u64().unwrap(), 7);
}

/// A2: hostile payloads verbatim through from_wire/as_str/dispatch;
/// `"ping"` still reaches the Ping arm.
#[test]
fn other_never_normalized() {
    let report = check_case("other_never_normalized");
    assert!(report.metrics["verbatim"].as_bool().unwrap());
}

/// Zero privileged dispatches from unknown variants; the exact
/// `"subscribe"` spelling still reaches the Subscribe arm.
#[test]
fn zero_privileged_dispatches() {
    let report = check_case("zero_privileged_dispatches");
    let m = &report.metrics;
    assert_eq!(m["privileged_dispatches"].as_u64().unwrap(), 0);
    assert_eq!(m["default_dispatches"].as_u64().unwrap(), 7);
    assert!(m["control_reaches_subscribe"].as_bool().unwrap());
}

/// License gate: src/wire.rs and the task driver carry the maddada
/// attribution + source commit.
#[test]
fn license_header_present() {
    let report = check_case("license_header_present");
    assert_eq!(report.metrics["files_checked"].as_u64().unwrap(), 2);
}
