//! tests/task_46.rs — context pressure compaction (task-46, rust).
//!
//! Four integration tests, 50/50 validation/adversarial:
//!
//! - V1 `under_pressure_the_engine_stays_quiet`: no candidates below the
//!   economic threshold; `compact` is false and the plan is empty.
//! - V2 `over_pressure_oldest_complete_unpinned_go_first`: over both
//!   pressures, completed unpinned steps are selected oldest-first and
//!   the plan output is bounded.
//! - A1 `adversarial_active_reasoning_is_never_torn`: pressure while all
//!   steps are in-flight — the completed-only eligibility rule keeps
//!   active reasoning out of the candidate set and the plan refuses.
//! - A2 `adversarial_maximum_pressure_and_task_level_pass`: a hostile
//!   mix selects exactly the safe subset, then the task driver runs
//!   end-to-end and passes.

use phlow_gauntlet::tasks::task_46::{self, DriverError};
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` from the live crate directory, like the other gauntlet
/// tests do.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-46-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: rust task, no nvim involved"),
        PathBuf::from("unused: rust task, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

#[test]
fn under_pressure_the_engine_stays_quiet() {
    let report = task_46::run_case("under_budget_nothing_compacted")
        .expect("V1 case must run to completion");
    assert!(report.passed, "V1 failed: {:?}", report.failures);
    // The engine still lists the two eligible candidates as ADVISORY
    // (the plan is computed regardless), but the pressure gate refuses
    // to compact: compact=false means no action on them.
    assert_eq!(report.metrics["compact"], serde_json::json!(false));
    assert_eq!(report.metrics["candidate_count"], serde_json::json!(2));
}

#[test]
fn over_pressure_oldest_complete_unpinned_go_first() {
    let report = task_46::run_case("pressure_compacts_oldest_first_sustained")
        .expect("V2 case must run to completion");
    assert!(report.passed, "V2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["oldest_first"], serde_json::json!(true));
    assert!(
        report.metrics["final_candidates"]
            .as_u64()
            .expect("final_candidates is a number")
            > 0
    );
    assert!(
        report.metrics["max_candidates_seen"]
            .as_u64()
            .expect("max_candidates_seen is a number")
            <= 64
    );
}

#[test]
fn adversarial_active_reasoning_is_never_torn() {
    let report = task_46::run_case("critical_section_pressure_compacts_nothing")
        .expect("A1 case must run to completion");
    assert!(report.passed, "A1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["compact"], serde_json::json!(false));
    assert_eq!(report.metrics["candidate_count"], serde_json::json!(0));
}

#[test]
fn adversarial_maximum_pressure_and_task_level_pass() {
    let report = task_46::run_case("adversarial_size_mix_never_candidates_active")
        .expect("A2 case must run to completion");
    assert!(report.passed, "A2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["inflight_excluded"], serde_json::json!(true));
    assert_eq!(report.metrics["pinned_excluded"], serde_json::json!(true));
    assert_eq!(
        report.metrics["candidate_ids"],
        serde_json::json!([3, 4]),
        "only the two small completed steps may be candidates"
    );

    let outcome = task_46::run(&test_ctx());
    match outcome {
        TaskOutcome::Pass { evidence } => {
            assert!(
                evidence
                    .iter()
                    .any(|line| line.contains("evaluate_compaction"))
            );
        }
        other => panic!("task-46 should pass, got: {other:?}"),
    }
    let _: Option<DriverError> = None;
}
