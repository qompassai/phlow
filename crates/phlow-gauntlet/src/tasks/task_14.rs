//! task-14: evaluator budget exhaustion (rust).
//!
//! Drives phlow's real evaluator (`crates/phlow-experiment/src/evaluator.rs`)
//! with budgets too small for the work. The evaluator must fail closed: stop
//! when the budget is exhausted, report the exhaustion explicitly as
//! [`ExperimentError::BudgetExhausted`] (never a generic error, a hang, or a
//! silent partial result), leave no half-applied state, and refuse further
//! consumption after exhaustion.

use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_experiment::{
    ArtifactDigest, BudgetTracker, CheckRun, EvalStage, Evaluator, EvidenceBundle, ExperimentError,
    VerificationOutcome,
};

/// Task id.
pub const ID: &str = "task-14";
/// Human-readable name.
pub const NAME: &str = "evaluator budget exhaustion";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

// Named budgets, all with units.
const DEADLINE_MS: u64 = 600_000;
/// Tool calls granted: enough for one execute, not for the full pipeline.
const TINY_TOOL_CALLS: u64 = 1;
/// Output bytes granted: smaller than the overspend probe.
const TINY_OUTPUT_BYTES: u64 = 128;
/// Output bytes of the legitimate execute step.
const WORK_OUTPUT_BYTES: u64 = 64;
/// Output bytes of the overspend probe: exceeds [`TINY_OUTPUT_BYTES`].
const OVERSPEND_OUTPUT_BYTES: u64 = 200;

/// Attempt the task: two exhaustion phases against the real evaluator.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    let mut evidence = Vec::new();
    match drive(&mut evidence) {
        Ok(()) => TaskOutcome::Pass { evidence },
        Err((where_, how)) => TaskOutcome::Fail {
            where_,
            how,
            evidence,
        },
    }
}

/// Both phases must hold for the task to pass.
fn drive(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    exhaustion_before_the_work(evidence)?;
    terminal_refusal_after_exhaustion(evidence)?;
    Ok(())
}

/// Names a driver stage and renders an unexpected evaluator error against it.
fn at(step: &str, err: ExperimentError) -> (String, String) {
    (
        step.to_string(),
        format!("unexpected evaluator error: {err}"),
    )
}

/// Builds a fresh evaluator on the tiny budget. The budget is valid — only
/// too small for the work — so construction itself must succeed.
fn evaluator_with_tiny_budget() -> Result<Evaluator, String> {
    let budget = BudgetTracker::new(TINY_TOOL_CALLS, TINY_OUTPUT_BYTES, DEADLINE_MS)
        .map_err(|e| format!("valid tiny budget rejected at construction: {e}"))?;
    Ok(Evaluator::new(budget))
}

/// Renders the observable evaluator state: stage, remaining calls, used bytes.
fn snapshot(evaluator: &Evaluator) -> String {
    format!(
        "stage={} remaining_calls={} used_bytes={}",
        evaluator.stage().name(),
        evaluator.budget().tool_calls_remaining(),
        evaluator.budget().output_bytes_used()
    )
}

/// Phase 1: one `execute` asking for more output bytes than the budget grants
/// must fail with the explicit `BudgetExhausted` variant *before* consuming
/// anything — stage, remaining calls, and used bytes all unchanged.
fn exhaustion_before_the_work(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    let mut evaluator =
        evaluator_with_tiny_budget().map_err(|e| ("budget-setup".to_string(), e))?;
    evaluator.validate().map_err(|e| at("validate", e))?;
    evaluator.prepare().map_err(|e| at("prepare", e))?;
    let before = snapshot(&evaluator);

    match evaluator.execute(1, OVERSPEND_OUTPUT_BYTES) {
        Err(ExperimentError::BudgetExhausted {
            what: "output bytes",
        }) => {
            evidence.push(format!(
                "overspend execute(1,{OVERSPEND_OUTPUT_BYTES}) -> \
                 BudgetExhausted{{what=\"output bytes\"}}: \
                 \"output bytes budget exhausted; failing closed\""
            ));
        }
        other => {
            return Err((
                "execute".to_string(),
                format!("overspend did not fail closed with BudgetExhausted: {other:?}"),
            ));
        }
    }

    let after = snapshot(&evaluator);
    if before != after {
        return Err((
            "state".to_string(),
            format!("failed consume mutated evaluator state: {before} -> {after}"),
        ));
    }
    evidence.push(format!("state unchanged after refusal: {after}"));
    Ok(())
}

/// Builds a complete evidence bundle: the terminal-refusal phase must deny
/// success even when the evidence itself is complete.
fn complete_evidence() -> Result<EvidenceBundle, String> {
    let check = CheckRun::new("gauntlet-check", vec!["check".to_string()], true)
        .map_err(|e| format!("check fixture rejected: {e}"))?;
    let artifact = ArtifactDigest::new("gauntlet-artifact", "deadbeef")
        .map_err(|e| format!("artifact fixture rejected: {e}"))?;
    let verification = VerificationOutcome::new(true, vec!["host-coverage".to_string()])
        .map_err(|e| format!("verification fixture rejected: {e}"))?;
    EvidenceBundle::new(vec![check], vec![artifact], verification)
        .map_err(|e| format!("bundle fixture rejected: {e}"))
}

/// Phase 2: spend the whole budget on one legitimate execute, then `verify`
/// must refuse to report success on exhausted work — explicit
/// `BudgetExhausted`, and terminal: the evaluator can never reach
/// review/promote, and a second `verify` is refused identically (no
/// resurrection).
fn terminal_refusal_after_exhaustion(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    let mut evaluator =
        evaluator_with_tiny_budget().map_err(|e| ("budget-setup".to_string(), e))?;
    evaluator.validate().map_err(|e| at("validate", e))?;
    evaluator.prepare().map_err(|e| at("prepare", e))?;
    evaluator
        .execute(1, WORK_OUTPUT_BYTES)
        .map_err(|e| at("execute", e))?;
    evidence.push(format!(
        "execute(1,{WORK_OUTPUT_BYTES}) ok; budget fully spent: {}",
        snapshot(&evaluator)
    ));
    assert_eq!(evaluator.stage(), EvalStage::Verify);

    let bundle = complete_evidence().map_err(|e| ("evidence".to_string(), e))?;
    assert!(bundle.is_complete());

    match evaluator.verify(&bundle) {
        Err(ExperimentError::BudgetExhausted { what: "tool calls" }) => {
            evidence.push(
                "verify(complete evidence) -> BudgetExhausted{what=\"tool calls\"}: \
                 \"tool calls budget exhausted; failing closed\" — \
                 refusing success on exhausted work"
                    .to_string(),
            );
        }
        other => {
            return Err((
                "verify".to_string(),
                format!("exhausted evaluator did not refuse verify: {other:?}"),
            ));
        }
    }

    // Resurrection attempt: a second verify is refused the same way, and the
    // observable state is identical — exhaustion is terminal, not a phase.
    let before = snapshot(&evaluator);
    match evaluator.verify(&bundle) {
        Err(ExperimentError::BudgetExhausted { what: "tool calls" }) => {
            evidence.push("second verify refused identically; no resurrection".to_string());
        }
        other => {
            return Err((
                "verify-resurrection".to_string(),
                format!("second verify on exhausted budget behaved differently: {other:?}"),
            ));
        }
    }
    let after = snapshot(&evaluator);
    if before != after {
        return Err((
            "state".to_string(),
            format!("refused verify mutated evaluator state: {before} -> {after}"),
        ));
    }
    evidence.push(format!(
        "terminal: {} — review/promote unreachable, no success reported",
        after
    ));
    Ok(())
}
