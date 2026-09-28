//! Integration tests for task-14: evaluator budget exhaustion.
//!
//! 50/50 split: 2 validation (adequate budget completes with spend
//! accounted; tiny budget fails closed before doing the work) + 2
//! adversarial (zero budget rejected at start, nothing executed;
//! consume-after-exhaustion rejected, no resurrection). A fifth test drives
//! the task's own `run` and asserts its evidence. Everything runs against
//! phlow's real evaluator (`phlow-experiment/src/evaluator.rs`) — no mocks
//! in the budget logic.

use phlow_experiment::{
    ArtifactDigest, BudgetTracker, CheckRun, EvalStage, Evaluator, EvidenceBundle, ExperimentError,
    VerificationOutcome,
};
use phlow_gauntlet::tasks::task_14;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Deadline used by every budget in this file (ms on the tracker's injected
/// clock). Large: these tests probe budget exhaustion, not time.
const DEADLINE_MS: u64 = 600_000;

/// A complete evidence bundle fixture: one passed check with argv, one
/// artifact digest, a verified outcome with coverage.
fn complete_evidence() -> EvidenceBundle {
    let check = CheckRun::new("gauntlet-check", vec!["check".to_string()], true)
        .expect("check fixture must be valid");
    let artifact = ArtifactDigest::new("gauntlet-artifact", "deadbeef")
        .expect("artifact fixture must be valid");
    let verification = VerificationOutcome::new(true, vec!["host-coverage".to_string()])
        .expect("verification fixture must be valid");
    let bundle = EvidenceBundle::new(vec![check], vec![artifact], verification)
        .expect("bundle fixture must be valid");
    assert!(bundle.is_complete(), "fixture bundle must be complete");
    bundle
}

/// Build a `Ctx` for the driver test. The Rust-kind driver never touches
/// nvim or diver, so placeholder paths keep the test hermetic.
fn driver_ctx() -> Ctx {
    let work_dir = std::env::temp_dir().join("gauntlet-task-14-driver");
    Ctx::new(
        PathBuf::from("/nonexistent/nvim"),
        PathBuf::from("/nonexistent/diver"),
        work_dir,
    )
    .expect("Ctx::new accepts non-empty paths")
}

/// V: an adequate budget runs the whole pipeline and the spend is accounted
/// exactly — remaining calls and used bytes match the declared costs.
#[test]
fn adequate_budget_completes_with_spend_accounted() {
    let mut evaluator =
        Evaluator::new(BudgetTracker::new(8, 4096, DEADLINE_MS).expect("adequate budget"));
    evaluator.validate().expect("validate");
    evaluator.prepare().expect("prepare");
    assert_eq!(evaluator.stage(), EvalStage::Execute);

    evaluator.execute(2, 300).expect("execute within budget");
    assert_eq!(evaluator.budget().tool_calls_remaining(), 6);
    assert_eq!(evaluator.budget().output_bytes_used(), 300);

    let bundle = complete_evidence();
    assert!(
        evaluator
            .verify(&bundle)
            .expect("verify with complete evidence"),
        "complete evidence with live budget must verify"
    );
    assert_eq!(evaluator.stage(), EvalStage::Review);

    evaluator.review().expect("review");
    evaluator.promote().expect("promote");
    assert_eq!(evaluator.stage(), EvalStage::Promote);

    // Spend accounted after the full pipeline: exactly the declared costs.
    assert_eq!(evaluator.budget().tool_calls_remaining(), 6);
    assert_eq!(evaluator.budget().output_bytes_used(), 300);

    // Promotion is terminal: no second promotion.
    match evaluator.promote() {
        Err(ExperimentError::BadStageOrder { .. }) => {}
        other => panic!("second promote must be BadStageOrder, got {other:?}"),
    }
}

/// V: a budget too small for the work fails closed *before* doing the work —
/// the error is the explicit `BudgetExhausted` variant (named dimension,
/// fail-closed message), and the failed consume mutates nothing.
#[test]
fn tiny_budget_fails_closed_before_doing_the_work() {
    let mut evaluator =
        Evaluator::new(BudgetTracker::new(2, 128, DEADLINE_MS).expect("tiny budget"));
    evaluator.validate().expect("validate");
    evaluator.prepare().expect("prepare");

    let err = match evaluator.execute(2, 200) {
        Err(err) => err,
        Ok(()) => panic!("overspend execute(2,200) against a (2,128) budget must not succeed"),
    };
    assert_eq!(
        err,
        ExperimentError::BudgetExhausted {
            what: "output bytes"
        },
        "exhaustion must be the explicit named variant, not a generic error"
    );
    assert_eq!(
        err.to_string(),
        "output bytes budget exhausted; failing closed"
    );

    // No half-applied state: nothing consumed, stage did not advance.
    assert_eq!(evaluator.stage(), EvalStage::Execute);
    assert_eq!(evaluator.budget().tool_calls_remaining(), 2);
    assert_eq!(evaluator.budget().output_bytes_used(), 0);
}

/// A: a zero budget in any dimension is rejected at construction — there is
/// no tracker to execute against, so nothing can run.
#[test]
fn zero_budget_rejected_at_start() {
    for (tool_calls, output_bytes, deadline_ms, field) in [
        (0, 128, DEADLINE_MS, "tool_calls_max"),
        (1, 0, DEADLINE_MS, "output_bytes_max"),
        (1, 128, 0, "deadline_ms"),
    ] {
        match BudgetTracker::new(tool_calls, output_bytes, deadline_ms) {
            Err(ExperimentError::InvalidBudget { field: got }) => assert_eq!(got, field),
            other => panic!("zero {field} must be InvalidBudget, got {other:?}"),
        }
    }
    let err = BudgetTracker::new(0, 128, DEADLINE_MS).expect_err("zero tool calls must fail");
    assert_eq!(err.to_string(), "tool_calls_max must be positive");
}

/// A: once exhausted, every further consume is rejected in every dimension —
/// exhaustion never resurrects, never partially applies, and checked
/// arithmetic means a wrap-around spend attempt fails closed instead of
/// looking small.
#[test]
fn consume_after_exhaustion_rejected_no_resurrection() {
    let mut tracker = BudgetTracker::new(1, 128, DEADLINE_MS).expect("budget");
    tracker.consume(1, 128).expect("exact spend to exhaustion");
    assert_eq!(tracker.tool_calls_remaining(), 0);
    assert_eq!(tracker.output_bytes_used(), 128);

    for (calls, bytes, what) in [
        (1, 0, "tool calls"),
        (0, 1, "output bytes"),
        (u64::MAX, u64::MAX, "tool calls"),
    ] {
        match tracker.consume(calls, bytes) {
            Err(ExperimentError::BudgetExhausted { what: got }) => assert_eq!(got, what),
            other => panic!(
                "post-exhaustion consume({calls},{bytes}) must stay exhausted, got {other:?}"
            ),
        }
    }
    // State identical: the refusals changed nothing.
    assert_eq!(tracker.tool_calls_remaining(), 0);
    assert_eq!(tracker.output_bytes_used(), 128);

    // Overflow probe: spending past u64::MAX output bytes fails closed via
    // checked addition, never wraps to zero.
    let mut wide = BudgetTracker::new(1, u64::MAX, DEADLINE_MS).expect("wide budget");
    wide.consume(1, u64::MAX).expect("spend to the exact max");
    match wide.consume(0, 1) {
        Err(ExperimentError::BudgetExhausted {
            what: "output bytes",
        }) => {}
        other => panic!("wrapping consume must fail closed, got {other:?}"),
    }
    assert_eq!(wide.output_bytes_used(), u64::MAX);
}

/// The task driver itself: `run` must report Pass with evidence showing the
/// explicit exhaustion variants, the unchanged state, and the terminal
/// refusal.
#[test]
fn task_driver_reports_explicit_exhaustion() {
    let ctx = driver_ctx();
    let evidence = match task_14::run(&ctx) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-14 driver failed at '{where_}': {how}\nevidence:\n{}",
            evidence.join("\n")
        ),
    };
    let joined = evidence.join("\n");
    for needle in [
        "BudgetExhausted",
        "output bytes budget exhausted; failing closed",
        "tool calls budget exhausted; failing closed",
        "state unchanged after refusal",
        "no resurrection",
        "terminal:",
    ] {
        assert!(
            joined.contains(needle),
            "driver evidence must show '{needle}':\n{joined}"
        );
    }
}
