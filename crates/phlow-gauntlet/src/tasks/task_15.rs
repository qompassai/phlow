//! task-15: lifecycle illegal transitions (rust).
//!
//! Drives phlow's real candidate-lifecycle state machine —
//! `phlow_experiment::promotion::Lifecycle::transition` — through the full
//! legal transition table and a battery of illegal moves. Every legal
//! (state, event) pair must succeed; every illegal pair must be rejected
//! with a typed error (`ExperimentError::BadTransition`, or
//! `ExperimentError::LifecycleTerminal` on the two terminal states), and
//! the state must be provably unchanged after each rejection.
//!
//! The unchanged-state property is structural, not just tested:
//! `transition` takes `self` by value over a `Copy` enum, so a rejection
//! cannot half-mutate the caller's state — the task-05 failure mode
//! (mutate before validate) is not representable here. The battery still
//! asserts the pre-call value afterward, so a future signature change
//! would fail loudly instead of silently.

use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_experiment::{ExperimentError, Lifecycle, LifecycleEvent};

/// Task id.
pub const ID: &str = "task-15";
/// Human-readable name.
pub const NAME: &str = "lifecycle illegal transitions";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Every legal (state, event, next-state) triple of the machine, in the
/// order of `Lifecycle::transition`'s match arms.
const LEGAL: [(Lifecycle, LifecycleEvent, Lifecycle); 13] = [
    (
        Lifecycle::Proposed,
        LifecycleEvent::ContractInvalid,
        Lifecycle::Rejected,
    ),
    (
        Lifecycle::Proposed,
        LifecycleEvent::WorkspaceCreated,
        Lifecycle::Isolated,
    ),
    (
        Lifecycle::Isolated,
        LifecycleEvent::ChecksComplete,
        Lifecycle::Tested,
    ),
    (
        Lifecycle::Isolated,
        LifecycleEvent::ChecksFailed,
        Lifecycle::Rejected,
    ),
    (
        Lifecycle::Tested,
        LifecycleEvent::EvaluationComplete,
        Lifecycle::Reviewed,
    ),
    (
        Lifecycle::Tested,
        LifecycleEvent::RegressionFound,
        Lifecycle::Rejected,
    ),
    (
        Lifecycle::Reviewed,
        LifecycleEvent::GatesSatisfied,
        Lifecycle::AwaitingHuman,
    ),
    (
        Lifecycle::Reviewed,
        LifecycleEvent::RegressionFound,
        Lifecycle::Rejected,
    ),
    (
        Lifecycle::AwaitingHuman,
        LifecycleEvent::HumanApproved,
        Lifecycle::Promoted,
    ),
    (
        Lifecycle::AwaitingHuman,
        LifecycleEvent::ApprovalDenied,
        Lifecycle::Rejected,
    ),
    (
        Lifecycle::AwaitingHuman,
        LifecycleEvent::ApprovalExpired,
        Lifecycle::Rejected,
    ),
    (
        Lifecycle::Promoted,
        LifecycleEvent::DeployedToCanary,
        Lifecycle::Monitored,
    ),
    (
        Lifecycle::Monitored,
        LifecycleEvent::RegressionDetected,
        Lifecycle::RolledBack,
    ),
];

/// Illegal (state, event) probes: `true` when the state is terminal, so a
/// `LifecycleTerminal` rejection is expected instead of `BadTransition`.
const ILLEGAL: [(Lifecycle, LifecycleEvent, bool); 8] = [
    // Skipped stages: evaluation can never run on a mere proposal.
    (
        Lifecycle::Proposed,
        LifecycleEvent::RegressionDetected,
        false,
    ),
    // Skipped gates: checks are not the promotion gate.
    (Lifecycle::Isolated, LifecycleEvent::GatesSatisfied, false),
    // Skipped review and human approval.
    (Lifecycle::Tested, LifecycleEvent::HumanApproved, false),
    // Deploying to canary without human approval.
    (
        Lifecycle::AwaitingHuman,
        LifecycleEvent::DeployedToCanary,
        false,
    ),
    // The task-brief's "completed -> running" analog: a promoted candidate
    // cannot regress; only canary deployment moves it forward.
    (
        Lifecycle::Promoted,
        LifecycleEvent::RegressionDetected,
        false,
    ),
    // Monitored candidates answer only to regression detection.
    (Lifecycle::Monitored, LifecycleEvent::HumanApproved, false),
    // Terminal states reject every event.
    (Lifecycle::Rejected, LifecycleEvent::WorkspaceCreated, true),
    (
        Lifecycle::RolledBack,
        LifecycleEvent::DeployedToCanary,
        true,
    ),
];

/// Attempt the task: legal table, terminal sequences, illegal battery.
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

/// Run the whole battery, appending evidence lines. Fails on the first
/// deviation: a legal move that errors, an illegal move that succeeds, a
/// wrong error variant, or a mutated state after rejection.
fn drive(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    // Phase 1 (validation): every legal transition succeeds.
    for (from, event, expected) in LEGAL {
        match from.transition(event) {
            Ok(next) if next == expected => evidence.push(format!(
                "ok: {} + {} -> {}",
                from.name(),
                event.name(),
                next.name()
            )),
            Ok(next) => {
                return Err((
                    "legal-transition".to_string(),
                    format!(
                        "wrong next state: {} + {} -> {}, expected {}",
                        from.name(),
                        event.name(),
                        next.name(),
                        expected.name()
                    ),
                ));
            }
            Err(err) => {
                return Err((
                    "legal-transition".to_string(),
                    format!(
                        "legal transition rejected: {} + {}: {err:?}",
                        from.name(),
                        event.name()
                    ),
                ));
            }
        }
    }

    // Phase 2 (validation): legal sequences reach each terminal state.
    let rejected = Lifecycle::Proposed.transition(LifecycleEvent::ContractInvalid);
    match rejected {
        Ok(state) if state == Lifecycle::Rejected && state.is_terminal() => {
            evidence.push("ok: proposed + contract_invalid reaches terminal rejected".to_string())
        }
        _ => {
            return Err((
                "terminal-sequence".to_string(),
                format!("rejection path did not reach terminal Rejected: {rejected:?}"),
            ));
        }
    }
    let mut state = Lifecycle::Proposed;
    for event in [
        LifecycleEvent::WorkspaceCreated,
        LifecycleEvent::ChecksComplete,
        LifecycleEvent::EvaluationComplete,
        LifecycleEvent::GatesSatisfied,
        LifecycleEvent::HumanApproved,
        LifecycleEvent::DeployedToCanary,
        LifecycleEvent::RegressionDetected,
    ] {
        state = state.transition(event).map_err(|err| {
            (
                "terminal-sequence".to_string(),
                format!(
                    "happy path broke at {} + {}: {err:?}",
                    state.name(),
                    event.name()
                ),
            )
        })?;
    }
    if state == Lifecycle::RolledBack && state.is_terminal() {
        evidence.push(
            "ok: proposed -> isolated -> tested -> reviewed -> awaiting_human \
             -> promoted -> monitored reaches terminal rolled_back"
                .to_string(),
        );
    } else {
        return Err((
            "terminal-sequence".to_string(),
            format!(
                "happy path did not end terminal RolledBack: {}",
                state.name()
            ),
        ));
    }

    // Phase 3+4 (adversarial): every illegal move is rejected, state
    // unchanged afterward.
    for (state, event, terminal) in ILLEGAL {
        expect_rejected(evidence, state, event, terminal)?;
    }
    Ok(())
}

/// Attempt one illegal move. Expect rejection with the right error variant
/// and prove the caller's state is unchanged afterward.
fn expect_rejected(
    evidence: &mut Vec<String>,
    state: Lifecycle,
    event: LifecycleEvent,
    terminal: bool,
) -> Result<(), (String, String)> {
    let before = state;
    match state.transition(event) {
        Ok(next) => Err((
            "illegal-transition".to_string(),
            format!(
                "ILLEGAL TRANSITION ACCEPTED: {} + {} -> {}",
                before.name(),
                event.name(),
                next.name()
            ),
        )),
        Err(err) => {
            let kind_ok = matches!(
                (&err, terminal),
                (ExperimentError::LifecycleTerminal { .. }, true)
                    | (ExperimentError::BadTransition { .. }, false)
            );
            if !kind_ok {
                return Err((
                    "illegal-transition".to_string(),
                    format!(
                        "wrong rejection for {} + {} (terminal={terminal}): {err:?}",
                        before.name(),
                        event.name()
                    ),
                ));
            }
            // By-value `self` over a `Copy` enum: the rejection provably
            // left the caller's state alone. Assert it for the record.
            if state != before {
                return Err((
                    "illegal-transition".to_string(),
                    format!(
                        "state mutated by rejected transition: {} + {}",
                        before.name(),
                        event.name()
                    ),
                ));
            }
            let kind = if terminal {
                "LifecycleTerminal"
            } else {
                "BadTransition"
            };
            evidence.push(format!(
                "reject: {} + {} -> {kind} (state still {})",
                before.name(),
                event.name(),
                before.name()
            ));
            Ok(())
        }
    }
}
