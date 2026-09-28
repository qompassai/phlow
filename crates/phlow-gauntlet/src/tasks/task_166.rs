// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 166 — pure transition function (rust, V).
//!
//! The seam is `reduce(state, event) -> (state, effects)` in
//! [`crate::state_machine`]: the reducer is pure — same inputs always
//! yield the same outputs — and effects are *recorded*, never executed
//! inside the reducer. Two scenarios: V1 replays a scripted 200-event
//! session twice and requires byte-identical final states and effect
//! lists; V2 statically asserts the reducer module imports zero I/O
//! facilities (no fs/net/clock) via a source scan of
//! `src/state_machine.rs`.
//!
//! Pattern adapted from Ghostex `packages/gx-core/src/core.rs`
//! ("events in, state plus effects out"); the domain (agent task
//! lifecycle for the TUI) is ours, Ghostex's tabs/sidebar model is not
//! lifted.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::state_machine::{Event, MachineState, canonical_effect_bytes, reduce};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-166";
/// Task name.
pub const NAME: &str = "pure transition function";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["replay_deterministic_200_events", "reducer_imports_no_io"];

/// 25 scripted tasks, 8 events each: spawn, start, progress(25),
/// progress(50), block, unblock, progress(100), complete (even ids) /
/// fail (odd ids). 200 events total, all transition-legal.
fn scripted_session() -> Vec<Event> {
    let mut events = Vec::with_capacity(200);
    for id in 1u64..=25 {
        events.push(Event::Spawn {
            id,
            label: format!("scripted-task-{id}"),
        });
        events.push(Event::Start { id });
        events.push(Event::Progress { id, percent: 25 });
        events.push(Event::Progress { id, percent: 50 });
        events.push(Event::Block {
            id,
            reason: "waiting on fixture".to_string(),
        });
        events.push(Event::Unblock { id });
        events.push(Event::Progress { id, percent: 100 });
        if id % 2 == 0 {
            events.push(Event::Complete { id });
        } else {
            events.push(Event::Fail {
                id,
                reason: "scripted failure".to_string(),
            });
        }
    }
    events
}

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// V1: the 200-event scripted session runs twice from a fresh state.
/// Final states and the concatenated effect lists must be
/// byte-identical (via the canonical encodings), and non-trivial
/// (25 tasks, version 200, effects recorded on every event).
/// One replay of the scripted session. Returns (failures, state
/// bytes, effect bytes). Non-triviality is checked here so a
/// byte-equality claim can never be vacuous.
fn replay_once(events: &[Event], run: usize) -> (Vec<String>, Vec<u8>, Vec<u8>) {
    let mut failures = Vec::new();
    let mut state = MachineState::default();
    let mut all_effects: Vec<u8> = Vec::new();
    for (i, event) in events.iter().enumerate() {
        match reduce(&state, event) {
            Ok((next, effects)) => {
                state = next;
                all_effects.extend_from_slice(&canonical_effect_bytes(&effects));
            }
            Err(e) => {
                failures.push(format!("run {run} event {i}: reduce failed: {e:?}"));
                break;
            }
        }
    }
    if state.tasks.len() != 25 {
        failures.push(format!(
            "run {run}: {} tasks in final state, want 25",
            state.tasks.len()
        ));
    }
    if state.version != 200 {
        failures.push(format!(
            "run {run}: version {}, want 200 (one per event)",
            state.version
        ));
    }
    if all_effects.is_empty() {
        failures.push(format!("run {run}: no effects recorded"));
    }
    (failures, state.canonical_bytes(), all_effects)
}

/// Assemble the V1 report from the two replay byte-strings.
fn replay_report(
    states_identical: bool,
    effects_identical: bool,
    state0: &[u8],
    effects0: &[u8],
    failures: Vec<String>,
) -> CaseReport {
    let mut evidence = vec![
        format!(
            "200-event scripted session x2: states byte-identical={states_identical} \
             ({} bytes), effects byte-identical={effects_identical} ({} bytes)",
            state0.len(),
            effects0.len()
        ),
        format!(
            "final state: 25 tasks, version 200; effect list non-empty: {}",
            !effects0.is_empty()
        ),
        "backend: pure reduce, no interpreter, no clock".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "events": 200,
            "replays": 2,
            "states_byte_identical": states_identical,
            "effects_byte_identical": effects_identical,
            "final_tasks": 25,
            "final_version": 200,
            "state_bytes": state0.len(),
            "effects_bytes": effects0.len(),
            "backend": "pure-reduce",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    report
}

fn case_replay_deterministic_200_events() -> Result<CaseReport, TaskDriverError> {
    let events = scripted_session();
    if events.len() != 200 {
        return Err(arm_error(
            "script",
            format!("scripted session has {} events, want 200", events.len()),
        ));
    }
    let mut failures = Vec::new();
    let (f0, state0, effects0) = replay_once(&events, 0);
    let (f1, state1, effects1) = replay_once(&events, 1);
    failures.extend(f0);
    failures.extend(f1);
    let states_identical = state0 == state1;
    let effects_identical = effects0 == effects1;
    if !states_identical {
        failures.push("final states differ between the two replays".to_string());
    }
    if !effects_identical {
        failures.push("effect lists differ between the two replays".to_string());
    }
    Ok(replay_report(
        states_identical,
        effects_identical,
        &state0,
        &effects0,
        failures,
    ))
}

/// I/O tokens that must not appear anywhere in the reducer module.
/// The mock interpreter lives in the same module and is itself free
/// of real I/O (in-memory logs only), so the scan covers the whole
/// file: purity is a property of the module, enforced statically.
const FORBIDDEN_IO_TOKENS: [&str; 12] = [
    "std::fs",
    "std::net",
    "std::io",
    "std::time",
    "std::process",
    "std::thread",
    "std::env",
    "SystemTime",
    "Instant",
    "TcpStream",
    "Command::",
    "std::os::",
];

/// V2: `src/state_machine.rs` is scanned for I/O facility imports.
/// Any hit fails the case: the reducer must not be able to perform
/// I/O, not merely promise not to.
fn case_reducer_imports_no_io() -> Result<CaseReport, TaskDriverError> {
    let source = include_str!("../state_machine.rs");
    let mut failures = Vec::new();
    let mut hits: Vec<&str> = Vec::new();
    for token in FORBIDDEN_IO_TOKENS {
        if source.contains(token) {
            hits.push(token);
            failures.push(format!("I/O token {token:?} present in state_machine.rs"));
        }
    }
    if !source.contains("pub fn reduce") {
        failures.push("state_machine.rs does not define `pub fn reduce`".to_string());
    }
    let mut evidence = vec![format!(
        "scanned src/state_machine.rs ({} lines) for {} I/O tokens: {} hits",
        source.lines().count(),
        FORBIDDEN_IO_TOKENS.len(),
        hits.len()
    )];
    if !hits.is_empty() {
        evidence.push(format!("hits: {hits:?}"));
    }
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "tokens_scanned": FORBIDDEN_IO_TOKENS.len(),
            "hits": hits,
            "reduce_defined": source.contains("pub fn reduce"),
            "backend": "static-source-scan",
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
        "replay_deterministic_200_events" => case_replay_deterministic_200_events(),
        "reducer_imports_no_io" => case_reducer_imports_no_io(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-166: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// replay determinism itself.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-166".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-166".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
