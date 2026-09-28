// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 168 — hostile event injection (rust, A).
//!
//! The seam is [`crate::state_machine::ingest`]: events crossing the
//! trust boundary (daemon → TUI) are validated before they may reach
//! [`crate::state_machine::reduce`]. Two scenarios: A1, an unknown
//! event variant is rejected at ingestion with
//! [`crate::state_machine::EventError::Unknown`] and the state is
//! untouched; A2, hostile payloads — a 10 MB string field, `u64::MAX`
//! and 0 as task ids, an out-of-range percent, an oversized reason —
//! are refused with typed errors, zero panics, and no allocation
//! spike (the 10 MB payload is refused on length before any copy; the
//! allocation delta is asserted under
//! [`crate::state_machine::INGEST_ALLOC_BUDGET_BYTES`]).
//!
//! Pattern adapted from Ghostex `packages/gx-core/src/core.rs` (its
//! event model); the validate-external-data layer is ours (Tiger
//! Style).

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::state_machine::{
    Event, EventError, MAX_LABEL_BYTES, MAX_REASON_BYTES, MachineState, RawEvent, ingest, reduce,
};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-168";
/// Task name.
pub const NAME: &str = "hostile event injection";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = [
    "unknown_event_variant_rejected",
    "hostile_payload_bounds_enforced",
];

/// Ten megabytes of hostile label.
const HOSTILE_LABEL_BYTES: usize = 10 * 1024 * 1024;

fn raw(kind: &str, task_id: u64, text: &str, percent: u64) -> RawEvent {
    RawEvent {
        kind: kind.to_string(),
        task_id,
        text: text.to_string(),
        percent,
    }
}

/// Reduce a benign event; panics on error (fixture script is legal by
/// construction).
fn apply(state: &MachineState, event: &Event) -> MachineState {
    match reduce(state, event) {
        Ok((next, _)) => next,
        Err(e) => panic!("task-168 fixture: reduce failed on {event:?}: {e:?}"),
    }
}

/// A1: unknown variants are rejected at ingestion — `EventError::Unknown`
/// naming the kind — and the state is byte-identical before and after
/// each attempt (reduce is never reached).
fn case_unknown_event_variant_rejected() -> Result<CaseReport, TaskDriverError> {
    let mut state = MachineState::default();
    state = apply(
        &state,
        &Event::Spawn {
            id: 1,
            label: "real task".to_string(),
        },
    );
    let before = state.canonical_bytes();
    let mut failures = Vec::new();
    let mut rejected = 0u32;
    for kind in [
        "defenestrate",
        "SPAWN",
        "spawn ",
        "",
        "complete\u{0}drop",
        "vendor:privileged-reset",
    ] {
        match ingest(&raw(kind, 1, "x", 0)) {
            Err(EventError::Unknown { kind: got }) if got == kind => {
                rejected += 1;
            }
            other => failures.push(format!("kind {kind:?}: wrong outcome: {other:?}")),
        }
        if state.canonical_bytes() != before {
            failures.push(format!("STATE CHANGED by unknown kind {kind:?}"));
        }
    }
    let mut evidence = vec![format!(
        "6 unknown variants -> EventError::Unknown each ({rejected}/6); \
         state byte-identical before/after every attempt"
    )];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "variants": 6,
            "rejected": rejected,
            "refusal": "Unknown",
            "state_unchanged": state.canonical_bytes() == before,
            "backend": "ingestion-boundary",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// The hostile ingestion batch. Pure assertions against [`ingest`];
/// never touches shared state. Run under `catch_unwind` by the case.
fn hostile_ingestion_batch(hostile_label: &str, hostile_reason: &str) -> Vec<String> {
    let mut local_failures: Vec<String> = Vec::new();
    // 10 MB label refused on length, before any copy into state.
    match ingest(&raw("spawn", 2, hostile_label, 0)) {
        Err(EventError::OversizedPayload { field, bytes, max })
            if field == "label" && bytes == HOSTILE_LABEL_BYTES && max == MAX_LABEL_BYTES => {}
        other => local_failures.push(format!("10MB label: wrong outcome: {other:?}")),
    }
    // u64::MAX and 0 as ids: reserved sentinels, never real tasks.
    for id in [u64::MAX, 0] {
        match ingest(&raw("spawn", id, "ok", 0)) {
            Err(EventError::OutOfRangeIndex { id: got }) if got == id => {}
            other => {
                local_failures.push(format!("id {id}: wrong outcome: {other:?}"));
            }
        }
    }
    // Percent out of range; reason over its bound.
    match ingest(&raw("progress", 7, "", 101)) {
        Err(EventError::BadPercent { percent: 101 }) => {}
        other => local_failures.push(format!("percent 101: wrong outcome: {other:?}")),
    }
    match ingest(&raw("fail", 7, hostile_reason, 0)) {
        Err(EventError::OversizedPayload { field, max, .. })
            if field == "reason" && max == MAX_REASON_BYTES => {}
        other => local_failures.push(format!("oversized reason: wrong outcome: {other:?}")),
    }
    // A hostile-but-well-formed event still works: bounds are
    // enforced, not weaponized against legitimate traffic.
    match ingest(&raw("start", 7, "", 0)) {
        Ok(Event::Start { id: 7 }) => {}
        other => local_failures.push(format!("benign start: wrong outcome: {other:?}")),
    }
    local_failures
}

/// A2: hostile payloads refused with typed errors. The whole batch
/// runs under `catch_unwind`: any panic anywhere is a case failure.
/// The 10 MB label is refused on length before any copy; the
/// allocation bound itself is asserted by the integration test, which
/// installs a real counting global allocator around `ingest`.
fn case_hostile_payload_bounds_enforced() -> Result<CaseReport, TaskDriverError> {
    let hostile_label = "x".repeat(HOSTILE_LABEL_BYTES);
    let hostile_reason = "y".repeat(MAX_REASON_BYTES + 1);
    let mut state = MachineState::default();
    state = apply(
        &state,
        &Event::Spawn {
            id: 7,
            label: "real task".to_string(),
        },
    );
    let before = state.canonical_bytes();
    let mut failures = Vec::new();

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hostile_ingestion_batch(&hostile_label, &hostile_reason)
    }));
    match outcome {
        Ok(local) => failures.extend(local),
        Err(_) => failures.push("PANIC during hostile ingestion batch".to_string()),
    }
    if state.canonical_bytes() != before {
        failures.push("STATE CHANGED during hostile ingestion".to_string());
    }
    let mut evidence = vec![
        format!(
            "10MB label -> OversizedPayload{{label, {HOSTILE_LABEL_BYTES}, {MAX_LABEL_BYTES}}}: \
             refused on length before any copy"
        ),
        "u64::MAX and 0 as task ids -> OutOfRangeIndex; percent 101 -> BadPercent; \
         oversized reason -> OversizedPayload{reason}"
            .to_string(),
        "zero panics across the hostile batch (catch_unwind); state byte-identical".to_string(),
        "allocation bound asserted by the integration test's counting allocator".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "refusals": ["OversizedPayload", "OutOfRangeIndex", "BadPercent"],
            "hostile_label_bytes": HOSTILE_LABEL_BYTES,
            "panics": 0,
            "state_unchanged": state.canonical_bytes() == before,
            "backend": "ingestion-boundary",
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
        "unknown_event_variant_rejected" => case_unknown_event_variant_rejected(),
        "hostile_payload_bounds_enforced" => case_hostile_payload_bounds_enforced(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-168: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// hostile-payload bounds.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[1]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-168".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-168".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
