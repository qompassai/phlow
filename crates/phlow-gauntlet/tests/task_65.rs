//! tests/task_65.rs — plan cost estimation (task-65, rust).
//!
//! Four integration tests, 50/50 validation/adversarial. The task
//! verdict is `fail` at `"seam"`: phlow has no plan-cost estimator —
//! the estimation vocabulary scan finds only the KV-cache control
//! sample, budget enforcement (`BudgetTracker` + `Evaluator`) is real
//! but estimation is absent, a doomed plan passes `validate()` and only
//! fails closed at `consume()`, and the estimate/enforcement unit-match
//! check is vacuous.
//!
//! - V1 `no_plan_cost_estimator`: no estimation hit outside the
//!   kv_policy.rs control sample.
//! - V2 `enforcement_without_estimation`: the real `Evaluator` walks
//!   Validate -> Prepare -> Execute with no Estimate stage.
//! - A1 `doomed_work_starts_at_enforcement`: a 100-call plan against a
//!   2-call budget passes validate(), fails at consume().
//! - A2 `units_match_check_vacuous`: enforced units are named; there is
//!   no estimate to compare them against.

use phlow_gauntlet::tasks::task_65;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` from the live crate directory, like the other gauntlet
/// tests do.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-65-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: rust task, no nvim involved"),
        PathBuf::from("unused: rust task, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

#[test]
fn no_plan_cost_estimator() {
    assert_eq!(task_65::ID, "task-65");
    assert_eq!(task_65::NAME, "plan cost estimation");
    assert_eq!(task_65::CASES.len(), 4, "2 validation + 2 adversarial");
    let report =
        task_65::run_case("no_plan_cost_estimator").expect("V1 case must run to completion");
    assert!(report.passed, "V1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["estimator_hits"], serde_json::json!(0));
}

#[test]
fn enforcement_without_estimation() {
    let report = task_65::run_case("enforcement_without_estimation")
        .expect("V2 case must run to completion");
    assert!(report.passed, "V2 failed: {:?}", report.failures);
    // 10-call budget, 3 consumed by the in-budget execute.
    assert_eq!(report.metrics["tool_calls_remaining"], serde_json::json!(7));
}

#[test]
fn doomed_work_starts_at_enforcement() {
    let report = task_65::run_case("doomed_work_starts_at_enforcement")
        .expect("A1 case must run to completion");
    assert!(report.passed, "A1 failed: {:?}", report.failures);
    assert_eq!(
        report.metrics["validate_passed_doomed"],
        serde_json::json!(true),
        "the doomed plan must pass validate() — no estimate gate"
    );
    assert_eq!(
        report.metrics["execute_rejected"],
        serde_json::json!(true),
        "enforcement must kill the doomed work at consume()"
    );
}

#[test]
fn units_match_check_vacuous() {
    let report =
        task_65::run_case("units_match_check_vacuous").expect("A2 case must run to completion");
    assert!(report.passed, "A2 failed: {:?}", report.failures);
    assert_eq!(
        report.metrics["estimate_units"],
        serde_json::Value::Null,
        "there is no estimate whose units could be compared"
    );
    assert_eq!(
        report.metrics["enforced_units"],
        serde_json::json!(["tool_calls", "output_bytes", "deadline_ms"])
    );

    // The task-level verdict is the honest fail at the absent seam (the
    // finding IS the gap, as the design allows) — a completed probe
    // finding, not a probe crash.
    let outcome = task_65::run(&test_ctx());
    match outcome {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-65 must fail at the absent seam");
            assert!(
                how.contains("no plan-cost estimator"),
                "the 'how' must name the missing estimator: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => {
            panic!("task-65 passed: an estimator was invented, not found\nevidence: {evidence:?}")
        }
    }
}
