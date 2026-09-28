//! tests/task_49.rs — CPU fairness (task-49, rust).
//!
//! Four integration tests, 50/50 validation/adversarial. The task
//! verdict is `fail` at `"seam"`: no fair task scheduler exists — no
//! worker pool, no preemption, no timeslices, no task-tree fairness —
//! so the design's round-robin policy has nowhere to live.
//!
//! - V1 `no_fairness_vocabulary_exists`: the fairness vocabulary scan
//!   returns zero, so the design's policy has nowhere to live.
//! - V2 `no_worker_pool_to_schedule`: no worker pool exists in any
//!   crate, so a scheduler cannot even reach its workers.
//! - A1 `deadlines_are_not_fairness`: no scheduler-shaped API exists
//!   among the adjacent budget/deadline APIs — a fair policy cannot be
//!   bolted onto a deadline.
//! - A2 `adversarial_task_level_verdict_is_fail_at_seam`: the driver
//!   reports the honest seam-absent failure.

use phlow_gauntlet::tasks::task_49::{self, DriverError};
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` from the live crate directory, like the other gauntlet
/// tests do.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-49-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: rust task, no nvim involved"),
        PathBuf::from("unused: rust task, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

#[test]
fn no_fairness_vocabulary_exists() {
    let report = task_49::run_case("no_scheduling_discipline_tokens")
        .expect("V1 case must run to completion");
    assert!(report.passed, "V1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["unexplained_hits"], serde_json::json!(0));
}

#[test]
fn no_worker_pool_to_schedule() {
    let report =
        task_49::run_case("no_worker_pool_to_schedule").expect("V2 case must run to completion");
    assert!(report.passed, "V2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["pool_hits"], serde_json::json!(0));
    assert_eq!(
        report.metrics["tokio_hits_outside_transport"],
        serde_json::json!(0)
    );
}

#[test]
fn deadlines_are_not_fairness() {
    let report = task_49::run_case("runaway_never_yields_has_no_target")
        .expect("A1 case must run to completion");
    assert!(report.passed, "A1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["schedulers_found"], serde_json::json!(0));
}

#[test]
fn adversarial_task_level_verdict_is_fail_at_seam() {
    let outcome = task_49::run(&test_ctx());
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(where_, "seam");
            assert!(how.contains("seam absent"));
            assert!(evidence.iter().any(|line| line.contains("fairness")));
        }
        other => panic!("task-49 should fail at the seam, got: {other:?}"),
    }
    let _: Option<DriverError> = None;
}
