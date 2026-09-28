// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-169 (effect ordering and idempotency).
//!
//! Three adversarial cases against
//! [`phlow_gauntlet::state_machine::MockInterpreter::deliver`]: a
//! duplicate keyed delivery is a typed no-op with zero double-applied
//! side effects; a keyless retry is refused loud; out-of-order
//! deliveries park in the bounded hold buffer and are rejected past
//! the bound.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::state_machine::HOLD_BUFFER_MAX;
use phlow_gauntlet::tasks::task_169;

fn check_case(case: &str) -> CaseReport {
    let report = task_169::run_case(case)
        .unwrap_or_else(|e| panic!("task-169 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-169 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- adversarial ---

/// A1: same keyed delivery twice → `Duplicate`; the ledger shows one
/// application per key — zero double-applied side effects.
#[test]
fn duplicate_delivery_idempotent() {
    assert_eq!(task_169::ID, "task-169");
    let report = check_case("duplicate_delivery_idempotent");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "Duplicate");
    assert_eq!(m["key"].as_str().unwrap(), "k-1");
    assert_eq!(m["ledger_entries"].as_u64().unwrap(), 2);
    assert_eq!(
        m["persist_task_1_applications"].as_u64().unwrap(),
        1,
        "the duplicated effect was double-applied"
    );
}

/// A2: a keyless effect presented as a retry → `RetryRefused`, not
/// double-applied.
#[test]
fn keyless_retry_refused() {
    let report = check_case("keyless_retry_refused");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "RetryRefused");
    assert_eq!(m["seq"].as_u64().unwrap(), 0);
    assert_eq!(
        m["notify_applications"].as_u64().unwrap(),
        1,
        "the retried keyless effect was double-applied"
    );
}

/// A3: out-of-order deliveries hold then drain in seq order; past
/// `HOLD_BUFFER_MAX` the interpreter rejects with `OutOfOrder`.
#[test]
fn out_of_order_bounded_hold() {
    let report = check_case("out_of_order_bounded_hold");
    let m = &report.metrics;
    assert_eq!(
        m["hold_buffer_max"].as_u64().unwrap(),
        HOLD_BUFFER_MAX as u64
    );
    assert_eq!(m["held"].as_u64().unwrap(), HOLD_BUFFER_MAX as u64);
    assert_eq!(m["refusal"].as_str().unwrap(), "OutOfOrder");
    assert!(
        m["ledger_ordered"].as_bool().unwrap(),
        "post-drain ledger must be gapless and in order"
    );
}
