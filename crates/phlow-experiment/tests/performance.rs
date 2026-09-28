//! Performance tests: default limits are sane, and a fixed-size no-op
//! workload completes exactly within its declared budget.
//!
//! Deterministic by construction: no wall-clock assertions, only exact
//! budget accounting.

#[path = "common/mod.rs"]
mod common;

use common::ok;
use phlow_experiment::{BudgetTracker, ExperimentError, SchedulerLimits};

#[test]
fn budget_constants_sane() {
    // Asserted on the runtime defaults, not the consts: asserting a
    // constant expression is a compile error on current nightly.
    let limits = SchedulerLimits::default();
    assert!(limits.workers_max > 0 && limits.workers_max <= 64);
    assert!(limits.queue_capacity > 0 && limits.queue_capacity <= 1_024);
    assert!(limits.depth_max > 0 && limits.depth_max <= 16);
    // Deadline between one second and one day, in milliseconds.
    assert!(limits.task_deadline_ms >= 1_000 && limits.task_deadline_ms <= 86_400_000);
    assert!(limits.aggregate_tool_calls_max > 0);
    assert!(limits.aggregate_output_bytes_max > 0);
    // The defaults validate as scheduler limits.
    ok(limits.validate());
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
