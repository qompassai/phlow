//! tests/task_50.rs — bounded event buffers (task-50, rust).
//!
//! Four integration tests, 50/50 validation/adversarial. The task
//! verdict is `fail` at `"seam"`: evict-style buffers exist and bound
//! memory, but none counts drops, no dropped-counter vocabulary exists,
//! and there is no producer/consumer event bus — so the design's
//! "memory bounded AND the dropped counter exact AND no silent loss"
//! pass criteria cannot be met.
//!
//! - V1 `evict_buffers_drop_silently`: the dropped-counter vocabulary
//!   scan returns zero and the real `Mailbox` carries no drop-counting
//!   field or method.
//! - V2 `no_producer_consumer_event_bus`: the event-bus vocabulary scan
//!   returns zero — the design's buffer has no bus to live on.
//! - A1 `sustained_overflow_loss_is_silent`: 600 sends against the real
//!   mailbox ring; 344 vanish, inferable only from id gaps.
//! - A2 `critical_kinds_are_dropped_too`: an early `Ask` record is
//!   evicted by a later flood of `Notice`s — no priority policy.

use phlow_gauntlet::tasks::task_50::{self, DriverError};
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` from the live crate directory, like the other gauntlet
/// tests do.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-50-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: rust task, no nvim involved"),
        PathBuf::from("unused: rust task, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

#[test]
fn evict_buffers_drop_silently() {
    let report =
        task_50::run_case("evict_buffers_drop_silently").expect("V1 case must run to completion");
    assert!(report.passed, "V1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["counter_hits"], serde_json::json!(0));
}

#[test]
fn no_producer_consumer_event_bus() {
    let report = task_50::run_case("no_producer_consumer_event_bus")
        .expect("V2 case must run to completion");
    assert!(report.passed, "V2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["event_bus_hits"], serde_json::json!(0));
}

#[test]
fn sustained_overflow_loss_is_silent() {
    let report = task_50::run_case("sustained_overflow_loss_is_silent")
        .expect("A1 case must run to completion");
    assert!(report.passed, "A1 failed: {:?}", report.failures);
    assert_eq!(
        report.metrics["dropped_counter_exposed"],
        serde_json::json!(false)
    );
    assert_eq!(report.metrics["ring_bound"], serde_json::json!(256));
    assert_eq!(
        report.metrics["inferred_dropped"],
        serde_json::json!(600 - 256)
    );
}

#[test]
fn critical_kinds_are_dropped_too() {
    let report = task_50::run_case("critical_kinds_are_dropped_too")
        .expect("A2 case must run to completion");
    assert!(report.passed, "A2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["ask_evicted"], serde_json::json!(true));
    assert_eq!(report.metrics["priority_policy"], serde_json::json!(false));

    let outcome = task_50::run(&test_ctx());
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(where_, "seam");
            assert!(how.contains("seam absent"));
            assert!(evidence.iter().any(|line| line.contains("dropped-counter")));
        }
        other => panic!("task-50 should fail at the seam, got: {other:?}"),
    }
    let _: Option<DriverError> = None;
}
