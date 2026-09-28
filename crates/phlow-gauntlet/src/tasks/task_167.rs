// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 167 — effect interpreter separation (rust, V).
//!
//! The seam is `interpret(effect)` in [`crate::state_machine`]:
//! effects execute only in the interpreter, in recorded order. An
//! effect kind with no interpreter clause is a typed
//! [`crate::state_machine::EffectError::Unhandled`] — never a silent
//! drop, never inline execution. Two scenarios: V1 runs
//! [Write, Notify, Write] through the mock interpreter and checks the
//! exact cross-sink application order; V2 feeds a batch containing an
//! unknown effect and checks the declared atomicity contract: the
//! whole batch is rejected, zero side effects recorded.
//!
//! Pattern adapted from Ghostex `packages/gx-core/src/core.rs` (the
//! `Effect` enum is host-executed there); the atomicity contract is
//! ours.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::state_machine::{Effect, EffectError, MachineState, MockInterpreter, reduce};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-167";
/// Task name.
pub const NAME: &str = "effect interpreter separation";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = [
    "effects_execute_in_recorded_order",
    "unknown_effect_atomic_batch_reject",
];

/// Reduce one event, appending its effects to `out`. Panics on
/// reducer error: the script is transition-legal by construction, so
/// an error here is a broken fixture, not a case finding.
fn step(state: &mut MachineState, event: crate::state_machine::Event, out: &mut Vec<Effect>) {
    match reduce(state, &event) {
        Ok((next, effects)) => {
            *state = next;
            out.extend(effects);
        }
        Err(e) => panic!("task-167 fixture: reduce failed on {event:?}: {e:?}"),
    }
}

/// The [Write, Notify, Write] script: two tasks through
/// spawn/start/complete. Returns every effect the reducer recorded,
/// in emission order.
fn recorded_order_script() -> Vec<Effect> {
    let mut state = MachineState::default();
    let mut effects: Vec<Effect> = Vec::new();
    use crate::state_machine::Event as E;
    step(
        &mut state,
        E::Spawn {
            id: 1,
            label: "one".into(),
        },
        &mut effects,
    );
    step(&mut state, E::Start { id: 1 }, &mut effects);
    step(&mut state, E::Complete { id: 1 }, &mut effects);
    step(
        &mut state,
        E::Spawn {
            id: 2,
            label: "two".into(),
        },
        &mut effects,
    );
    step(&mut state, E::Start { id: 2 }, &mut effects);
    effects
}

/// V1: reducer emits [Write, Notify, Write] (persist 1, notify 1,
/// persist 2 across two tasks); the interpreter must execute them in
/// exactly that recorded order, observed on the cross-sink sequence
/// log — not just per-sink.
fn case_effects_execute_in_recorded_order() -> Result<CaseReport, TaskDriverError> {
    let effects = recorded_order_script();
    let mut failures = Vec::new();
    if effects.len() != 6 {
        failures.push(format!("reducer emitted {} effects, want 6", effects.len()));
    }
    let mut interp = MockInterpreter::new();
    if let Err(e) = interp.interpret_batch(&effects) {
        failures.push(format!("interpret_batch failed: {e:?}"));
    }
    // The recorded order: persist 1 (spawn), persist 1 (start),
    // persist 1 + notify 1 (complete), persist 2 (spawn), persist 2
    // (start). The [Write, Notify, Write] core is entries 2..=4.
    let want_sequence = [
        "fs:persist task 1",
        "fs:persist task 1",
        "fs:persist task 1",
        "notify:notify task 1: task 1 done",
        "fs:persist task 2",
        "fs:persist task 2",
    ];
    if interp.sequence != want_sequence {
        failures.push(format!(
            "application order wrong:\n  got:  {:?}\n  want: {:?}",
            interp.sequence, want_sequence
        ));
    }
    let mut evidence = vec![
        format!(
            "6 reducer effects interpreted; cross-sink sequence has {} entries, order exact",
            interp.sequence.len()
        ),
        format!("fs mock observed {} writes", interp.fs_log.len()),
        format!(
            "notify mock observed {} notifications",
            interp.notify_log.len()
        ),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "effects": effects.len(),
            "sequence": interp.sequence,
            "fs_writes": interp.fs_log.len(),
            "notifies": interp.notify_log.len(),
            "backend": "mock-interpreter",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// V2: the batch [Persist, Notify, VendorExtension] must be rejected
/// atomically: `EffectError::Unhandled`, and — the load-bearing half
/// of the contract — zero side effects recorded on any mock (the
/// Persist and Notify ahead of the unknown effect must NOT have run).
/// A lone unknown effect interpreted directly is refused the same way.
fn case_unknown_effect_atomic_batch_reject() -> Result<CaseReport, TaskDriverError> {
    let batch = vec![
        Effect::PersistTask { task_id: 1 },
        Effect::Notify {
            task_id: 1,
            message: "task 1 done".to_string(),
        },
        Effect::VendorExtension {
            name: "future-effect".to_string(),
        },
    ];
    let mut interp = MockInterpreter::new();
    let mut failures = Vec::new();
    match interp.interpret_batch(&batch) {
        Err(EffectError::Unhandled { name }) if name == "future-effect" => {}
        other => failures.push(format!(
            "batch with unknown effect: wrong outcome: {other:?}"
        )),
    }
    if !interp.fs_log.is_empty() {
        failures.push(format!(
            "ATOMICITY BROKEN: {} fs writes applied before the rejection",
            interp.fs_log.len()
        ));
    }
    if !interp.notify_log.is_empty() {
        failures.push(format!(
            "ATOMICITY BROKEN: {} notifies applied before the rejection",
            interp.notify_log.len()
        ));
    }
    if !interp.sequence.is_empty() {
        failures.push("ATOMICITY BROKEN: sequence log non-empty after rejection".to_string());
    }
    // A lone unknown effect is refused identically, not silently dropped.
    let mut interp2 = MockInterpreter::new();
    match interp2.interpret_batch(&batch[2..]) {
        Err(EffectError::Unhandled { name }) if name == "future-effect" => {}
        other => failures.push(format!("lone unknown effect: wrong outcome: {other:?}")),
    }
    let mut evidence = vec![
        "batch [Persist, Notify, VendorExtension(future-effect)] -> \
         EffectError::Unhandled{future-effect}; fs/notify/sequence logs all empty: \
         atomic batch reject holds"
            .to_string(),
        "lone VendorExtension -> the same typed refusal (never a silent drop)".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "refusal": "Unhandled",
            "unknown_name": "future-effect",
            "fs_writes_after_reject": interp.fs_log.len(),
            "notifies_after_reject": interp.notify_log.len(),
            "sequence_after_reject": interp.sequence.len(),
            "atomicity": "batch-reject",
            "backend": "mock-interpreter",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "effects_execute_in_recorded_order" => case_effects_execute_in_recorded_order(),
        "unknown_effect_atomic_batch_reject" => case_unknown_effect_atomic_batch_reject(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-167: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// recorded-order execution itself.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-167".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-167".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
