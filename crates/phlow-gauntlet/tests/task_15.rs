//! Integration tests for task-15: lifecycle illegal transitions.
//!
//! 50/50 split against phlow's real candidate-lifecycle state machine
//! (`phlow_experiment::promotion::Lifecycle::transition`).
//!
//! - V1: every legal transition in the happy path succeeds — `run()`
//!   reports Pass with all 13 legal moves evidenced.
//! - V2: legal transition sequences reach each terminal state (Rejected
//!   and RolledBack).
//! - A1: the "completed -> running" analog — `Promoted +
//!   RegressionDetected` — is rejected with `BadTransition`, and the
//!   state is still `Promoted` afterward.
//! - A2: a battery of 7 illegal transitions is rejected (terminal states
//!   with `LifecycleTerminal`, the rest with `BadTransition`), asserting
//!   the state is unchanged after each rejection.

use phlow_experiment::{ExperimentError, Lifecycle, LifecycleEvent};
use phlow_gauntlet::tasks::task_15;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` for one test. This task drives no Neovim and no nvim-lua
/// driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("unused: task-15 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-15 is TaskKind::Rust, no diver lua involved"),
        std::env::temp_dir().join("gauntlet-task-15"),
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

/// Run the driver; unwrap the Pass outcome or fail with the driver's own
/// evidence attached.
fn run_pass() -> Vec<String> {
    match task_15::run(&test_ctx()) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-15 driver failed at {where_}: {how}\nevidence: {evidence:?}"),
    }
}

/// V1: every legal transition in the happy path succeeds.
#[test]
fn v_all_legal_transitions_succeed() {
    let evidence = run_pass();
    let legal_ok = evidence
        .iter()
        .filter(|line| {
            line.starts_with("ok: ") && line.contains(" + ") && !line.contains("reaches")
        })
        .count();
    assert_eq!(
        legal_ok, 13,
        "expected 13 legal-transition evidence lines, got {legal_ok}: {evidence:?}"
    );
}

/// V2: legal transition sequences reach each terminal state.
#[test]
fn v_sequences_reach_each_terminal_state() {
    let evidence = run_pass();
    let joined = evidence.join("\n");
    assert!(
        joined.contains("terminal rejected"),
        "no evidence of reaching terminal Rejected: {joined}"
    );
    assert!(
        joined.contains("terminal rolled_back"),
        "no evidence of reaching terminal RolledBack: {joined}"
    );
}

/// A1: completed -> running analog is rejected; state still completed.
/// `Promoted` is the machine's "completed" stage; `RegressionDetected` is
/// an event that only makes sense from `Monitored`.
#[test]
fn a_completed_to_running_rejected_state_unchanged() {
    let state = Lifecycle::Promoted;
    match state.transition(LifecycleEvent::RegressionDetected) {
        Ok(next) => panic!(
            "illegal transition accepted: promoted + regression_detected -> {}",
            next.name()
        ),
        Err(ExperimentError::BadTransition { from, event }) => {
            assert_eq!(from, "promoted");
            assert_eq!(event, "regression_detected");
        }
        Err(other) => panic!("wrong rejection variant: {other:?}"),
    }
    assert_eq!(state, Lifecycle::Promoted, "state mutated by rejection");
}

/// Assert one illegal move is rejected with the expected variant and the
/// state is unchanged afterward.
fn assert_rejected_unchanged(state: Lifecycle, event: LifecycleEvent, terminal: bool) {
    let before = state;
    match state.transition(event) {
        Ok(next) => panic!(
            "illegal transition accepted: {} + {} -> {}",
            before.name(),
            event.name(),
            next.name()
        ),
        Err(err) => {
            match (&err, terminal) {
                (ExperimentError::LifecycleTerminal { .. }, true) => {}
                (ExperimentError::BadTransition { .. }, false) => {}
                _ => panic!(
                    "wrong rejection for {} + {} (terminal={terminal}): {err:?}",
                    before.name(),
                    event.name()
                ),
            }
            assert_eq!(
                state,
                before,
                "state mutated by rejected transition {} + {}",
                before.name(),
                event.name()
            );
        }
    }
}

/// A2: a battery of 7 illegal transitions is rejected, state unchanged
/// after each.
#[test]
fn a_illegal_transition_battery_rejected_state_unchanged() {
    assert_rejected_unchanged(
        Lifecycle::Proposed,
        LifecycleEvent::RegressionDetected,
        false,
    );
    assert_rejected_unchanged(Lifecycle::Tested, LifecycleEvent::HumanApproved, false);
    assert_rejected_unchanged(
        Lifecycle::AwaitingHuman,
        LifecycleEvent::DeployedToCanary,
        false,
    );
    assert_rejected_unchanged(
        Lifecycle::Promoted,
        LifecycleEvent::RegressionDetected,
        false,
    );
    assert_rejected_unchanged(Lifecycle::Monitored, LifecycleEvent::HumanApproved, false);
    assert_rejected_unchanged(Lifecycle::Rejected, LifecycleEvent::WorkspaceCreated, true);
    assert_rejected_unchanged(
        Lifecycle::RolledBack,
        LifecycleEvent::DeployedToCanary,
        true,
    );
}

/// Task metadata (ID/NAME/KIND) is intact.
#[test]
fn task_metadata_intact() {
    assert_eq!(task_15::ID, "task-15");
    assert_eq!(task_15::NAME, "lifecycle illegal transitions");
    assert!(matches!(task_15::KIND, TaskKind::Rust));
}
