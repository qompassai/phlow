//! tests/task_63.rs — decomposition depth bound (task-63, rust).
//!
//! Four integration tests, 50/50 validation/adversarial. The task
//! verdict is `fail` at `"seam"`: no goal-decomposition routine exists in
//! any phlow crate — the decomposition vocabulary scan returns zero,
//! `phlow-agent`'s public module list has no planner/decomposer module,
//! and the "planner" in phlow is an LLM role name (config) plus prose,
//! never a routine that splits goals into subgoals.
//!
//! - V1 `decompose_vocabulary_absent`: the decomposition vocabulary
//!   scan returns zero *goal*-decomposition hits — the single hit is a
//!   verified control sample (mixed-radix tensor-index decomposition,
//!   classified UNRELATED by line content).
//! - V2 `planner_is_role_not_routine`: every "planner" hit file is
//!   verified as the planner role by the coder/reviewer sibling check
//!   — no routine.
//! - A1 `self_similar_goal_has_no_decomposer`: the design's adversarial
//!   self-similar goals have no decomposer to loop in (live module
//!   list + vocabulary scan).
//! - A2 `fail_closed_if_decomposer_appears`: the union scan is clean and
//!   the fail-closed branch is armed.

use phlow_gauntlet::tasks::task_63;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` from the live crate directory, like the other gauntlet
/// tests do.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-63-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: rust task, no nvim involved"),
        PathBuf::from("unused: rust task, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

#[test]
fn decompose_vocabulary_absent() {
    assert_eq!(task_63::ID, "task-63");
    assert_eq!(task_63::NAME, "decomposition depth bound");
    assert_eq!(task_63::CASES.len(), 4, "2 validation + 2 adversarial");
    let report =
        task_63::run_case("decompose_vocabulary_absent").expect("V1 case must run to completion");
    assert!(report.passed, "V1 failed: {:?}", report.failures);
    // Zero REAL goal-decomposition hits; the one hit is the verified
    // mixed-radix control sample (classified, not counted).
    assert_eq!(report.metrics["real"], serde_json::json!(0));
    assert!(
        report.metrics["control_samples"].as_u64().unwrap_or(0) > 0,
        "the mixed-radix control sample must exist to be classified: {}",
        report.metrics
    );
}

#[test]
fn planner_is_role_not_routine() {
    let report =
        task_63::run_case("planner_is_role_not_routine").expect("V2 case must run to completion");
    assert!(report.passed, "V2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["routine_hits"], serde_json::json!(0));
    // The planner-role hit files exist and are verified by the
    // coder/reviewer sibling check, not counted as routines.
    assert!(
        report.metrics["planner_files"].as_u64().unwrap_or(0) > 0,
        "the planner-role hit files must exist to be verified: {}",
        report.metrics
    );
}

#[test]
fn self_similar_goal_has_no_decomposer() {
    let report = task_63::run_case("self_similar_goal_has_no_decomposer")
        .expect("A1 case must run to completion");
    assert!(report.passed, "A1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["decompose_hits"], serde_json::json!(0));
    assert_eq!(
        report.metrics["adversarial_goals"],
        serde_json::json!(3),
        "the design's three self-similar goals must all be probed"
    );
}

#[test]
fn fail_closed_if_decomposer_appears() {
    let report = task_63::run_case("fail_closed_if_decomposer_appears")
        .expect("A2 case must run to completion");
    assert!(report.passed, "A2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["union_hits"], serde_json::json!(0));

    // The task-level verdict is the honest fail at the absent seam — a
    // completed probe finding, not a probe crash.
    let outcome = task_63::run(&test_ctx());
    match outcome {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-63 must fail at the absent seam");
            assert!(
                how.contains("no goal-decomposition routine exists"),
                "the 'how' must name the absent routine: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => {
            panic!("task-63 passed: a decomposer was invented, not found\nevidence: {evidence:?}")
        }
    }
}
