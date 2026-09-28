//! tests/task_48.rs — disk quota enforcement (task-48, rust).
//!
//! Four integration tests, 50/50 validation/adversarial. The task
//! verdict is `fail` at `"seam"`: no per-run quota, no `QuotaExceeded`,
//! no cumulative write accounting, and no artifact writer enforcing a
//! quota exist anywhere in the workspace.
//!
//! - V1 `no_quota_vocabulary_exists`: the quota vocabulary scan returns
//!   zero, so the design's typed refusal has nowhere to live.
//! - V2 `no_cumulative_write_accounting`: no writer tracks cumulative
//!   run bytes against a cap, so a per-run quota has nothing to debit.
//! - A1 `no_artifact_writer_enforces_a_quota`: no typed
//!   `QuotaExceeded`-shaped error exists for a mid-write failure.
//! - A2 `adversarial_task_level_verdict_is_fail_at_seam`: the driver
//!   reports the honest seam-absent failure.

use phlow_gauntlet::tasks::task_48::{self, DriverError};
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` from the live crate directory, like the other gauntlet
/// tests do.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-48-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: rust task, no nvim involved"),
        PathBuf::from("unused: rust task, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

#[test]
fn no_quota_vocabulary_exists() {
    let report =
        task_48::run_case("no_quota_tokens_in_sources").expect("V1 case must run to completion");
    assert!(report.passed, "V1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["quota_hits"], serde_json::json!(0));
}

#[test]
fn no_cumulative_write_accounting() {
    let report = task_48::run_case("write_paths_have_no_byte_accounting")
        .expect("V2 case must run to completion");
    assert!(report.passed, "V2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["accounting_hits"], serde_json::json!(0));
}

#[test]
fn no_artifact_writer_enforces_a_quota() {
    let report = task_48::run_case("mid_write_quota_exceeded_has_no_target")
        .expect("A1 case must run to completion");
    assert!(report.passed, "A1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["quota_exceeded_types"], serde_json::json!(0));
}

#[test]
fn adversarial_task_level_verdict_is_fail_at_seam() {
    let outcome = task_48::run(&test_ctx());
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(where_, "seam");
            assert!(how.contains("seam absent"));
            assert!(evidence.iter().any(|line| line.contains("per-run quota")));
        }
        other => panic!("task-48 should fail at the seam, got: {other:?}"),
    }
    let _: Option<DriverError> = None;
}
