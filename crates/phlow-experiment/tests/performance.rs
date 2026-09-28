//! Performance tests: budget constants are sane, and a fixed-size no-op
//! workload completes exactly within its declared budget.
//!
//! Deterministic by construction: no wall-clock assertions, only exact
//! budget accounting.

#[path = "common/mod.rs"]
mod common;

use common::ok;
use phlow_experiment::{
    AGGREGATE_OUTPUT_BYTES_MAX_DEFAULT, AGGREGATE_TOOL_CALLS_MAX_DEFAULT, BudgetTracker,
    DEPTH_MAX_DEFAULT, ExperimentError, QUEUE_CAPACITY_DEFAULT, SchedulerLimits,
    TASK_DEADLINE_MS_DEFAULT, WORKERS_MAX_DEFAULT,
};

#[test]
fn budget_constants_sane() {
    assert!(WORKERS_MAX_DEFAULT > 0 && WORKERS_MAX_DEFAULT <= 64);
    assert!(QUEUE_CAPACITY_DEFAULT > 0 && QUEUE_CAPACITY_DEFAULT <= 1_024);
    assert!(DEPTH_MAX_DEFAULT > 0 && DEPTH_MAX_DEFAULT <= 16);
    // Deadline between one second and one day, in milliseconds.
    assert!(TASK_DEADLINE_MS_DEFAULT >= 1_000 && TASK_DEADLINE_MS_DEFAULT <= 86_400_000);
    assert!(AGGREGATE_TOOL_CALLS_MAX_DEFAULT > 0);
    assert!(AGGREGATE_OUTPUT_BYTES_MAX_DEFAULT > 0);
    // The defaults validate as scheduler limits.
    ok(SchedulerLimits::default().validate());
}

#[test]
fn fixed_size_noop_workload_within_bounds() {
    // Fixed-size, deterministic, fast: exactly 4096 no-op iterations, each
    // consuming one tool call from a 4096-call budget.
    const ITERATIONS: u64 = 4_096;
    let mut tracker = ok(BudgetTracker::new(ITERATIONS, 1_048_576, 60_000));
    let mut checksum: u64 = 0;
    for _ in 0..ITERATIONS {
        ok(tracker.consume(1, 0));
        checksum = checksum.wrapping_add(1);
    }
    assert_eq!(checksum, ITERATIONS);
    assert_eq!(tracker.tool_calls_remaining(), 0);
    // The budget is exactly exhausted: one more call fails closed, never wraps.
    let over = tracker.consume(1, 0);
    assert!(matches!(over, Err(ExperimentError::BudgetExhausted { .. })));
    assert_eq!(tracker.tool_calls_remaining(), 0);
}
