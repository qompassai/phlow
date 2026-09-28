// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 169 — effect ordering and idempotency (rust, A).
//!
//! The seam is the effect log under replay/duplication, on the
//! transport path ([`crate::state_machine::MockInterpreter::deliver`]).
//! Three scenarios: A1, the same keyed effect delivered twice → the
//! second delivery is a typed no-op
//! ([`crate::state_machine::EffectError::Duplicate`]) with zero
//! double-applied side effects in the mock ledger; A2, a keyless
//! effect presented as a retry →
//! [`crate::state_machine::EffectError::RetryRefused`] (keyless
//! effects are declared non-idempotent — the interpreter refuses loud
//! rather than double-applying); A3, out-of-order deliveries park in
//! the bounded hold buffer until the gap fills, and past the bound
//! ([`crate::state_machine::HOLD_BUFFER_MAX`]) the interpreter
//! rejects with [`crate::state_machine::EffectError::OutOfOrder`].
//!
//! Pattern adapted from Ghostex `packages/gx-core/src/core.rs`
//! (effects-out); the idempotency-key and hold-buffer contracts are
//! ours.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::state_machine::{Delivery, Effect, EffectError, HOLD_BUFFER_MAX, MockInterpreter};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-169";
/// Task name.
pub const NAME: &str = "effect ordering and idempotency";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 3 adversarial.
pub const CASES: [&str; 3] = [
    "duplicate_delivery_idempotent",
    "keyless_retry_refused",
    "out_of_order_bounded_hold",
];

fn persist(seq: u64, key: Option<&str>, task_id: u64) -> Delivery {
    Delivery {
        seq,
        key: key.map(|k| k.to_string()),
        effect: Effect::PersistTask { task_id },
    }
}

fn notify(seq: u64, key: Option<&str>, task_id: u64, message: &str) -> Delivery {
    Delivery {
        seq,
        key: key.map(|k| k.to_string()),
        effect: Effect::Notify {
            task_id,
            message: message.to_string(),
        },
    }
}

/// A1: the same keyed delivery twice (daemon retry). The second is a
/// typed no-op `Duplicate`; the mock ledger holds exactly one entry
/// and the fs mock exactly one write — zero double-applied side
/// effects.
fn case_duplicate_delivery_idempotent() -> Result<CaseReport, TaskDriverError> {
    let mut interp = MockInterpreter::new();
    let mut failures = Vec::new();
    let first = persist(0, Some("k-1"), 1);
    if let Err(e) = interp.deliver(first, 0) {
        failures.push(format!("first delivery failed: {e:?}"));
    }
    match interp.deliver(persist(0, Some("k-1"), 1), 1) {
        Err(EffectError::Duplicate { key }) if key == "k-1" => {}
        other => failures.push(format!("duplicate delivery: wrong outcome: {other:?}")),
    }
    // A different key still applies: dedupe is per-key, not per-seq.
    if let Err(e) = interp.deliver(persist(1, Some("k-2"), 2), 0) {
        failures.push(format!("second key delivery failed: {e:?}"));
    }
    if interp.ledger.len() != 2 {
        failures.push(format!(
            "ledger has {} entries, want 2 (one per key)",
            interp.ledger.len()
        ));
    }
    let persist_writes = interp
        .fs_log
        .iter()
        .filter(|l| l.as_str() == "persist task 1")
        .count();
    if persist_writes != 1 {
        failures.push(format!(
            "DOUBLE-APPLY: 'persist task 1' written {persist_writes} times"
        ));
    }
    let mut evidence = vec![
        "keyed delivery (seq 0, key k-1) x2 -> second is Duplicate{k-1}: typed no-op".to_string(),
        format!(
            "ledger entries: {}; 'persist task 1' applied exactly once",
            interp.ledger.len()
        ),
        "distinct key k-2 applies normally: dedupe is per-key".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "refusal": "Duplicate",
            "key": "k-1",
            "ledger_entries": interp.ledger.len(),
            "persist_task_1_applications": persist_writes,
            "backend": "mock-interpreter-transport",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// A2: a keyless effect delivered, then presented again as a retry
/// (attempt=1). The interpreter refuses with `RetryRefused` — loud,
/// not double-applied. Keyless effects are declared non-idempotent:
/// the interpreter cannot dedupe what it cannot identify.
fn case_keyless_retry_refused() -> Result<CaseReport, TaskDriverError> {
    let mut interp = MockInterpreter::new();
    let mut failures = Vec::new();
    if let Err(e) = interp.deliver(notify(0, None, 1, "task 1 done"), 0) {
        failures.push(format!("first keyless delivery failed: {e:?}"));
    }
    match interp.deliver(notify(0, None, 1, "task 1 done"), 1) {
        Err(EffectError::RetryRefused { seq: 0 }) => {}
        other => failures.push(format!("keyless retry: wrong outcome: {other:?}")),
    }
    let notifies = interp
        .notify_log
        .iter()
        .filter(|l| l.contains("task 1 done"))
        .count();
    if notifies != 1 {
        failures.push(format!(
            "DOUBLE-APPLY: 'task 1 done' notified {notifies} times"
        ));
    }
    let mut evidence = vec![
        "keyless delivery (seq 0, attempt 0) applies; same effect as attempt 1 -> \
         RetryRefused{seq: 0}: loud refusal, zero double-apply"
            .to_string(),
        format!("notify log holds the message exactly {notifies}x"),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "refusal": "RetryRefused",
            "seq": 0,
            "notify_applications": notifies,
            "backend": "mock-interpreter-transport",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Gap fill: 0 applies, 2 parks, 1 fills the gap -> all apply in
/// order 0,1,2. Returns the failure list.
fn gap_fill_check() -> Vec<String> {
    let mut failures = Vec::new();
    let mut interp = MockInterpreter::new();
    for seq in [0u64, 2, 1] {
        if let Err(e) = interp.deliver(persist(seq, Some(&format!("g-{seq}")), seq + 1), 0) {
            failures.push(format!("gap-fill delivery seq {seq} failed: {e:?}"));
        }
    }
    let seqs: Vec<u64> = interp.ledger.iter().map(|e| e.seq).collect();
    if seqs != [0, 1, 2] {
        failures.push(format!("ledger order {seqs:?}, want [0, 1, 2]"));
    }
    failures
}

/// Past the bound: hold exactly HOLD_BUFFER_MAX, refuse the next with
/// `OutOfOrder`, then confirm the gap still fills and drains in
/// order. Returns (failures, held count, post-drain ledger seqs).
fn bound_overflow_check() -> (Vec<String>, u32, Vec<u64>) {
    let mut failures = Vec::new();
    let mut interp = MockInterpreter::new();
    if let Err(e) = interp.deliver(persist(0, Some("b-0"), 1), 0) {
        failures.push(format!("bound setup seq 0 failed: {e:?}"));
    }
    let mut held = 0u32;
    for seq in 2..=(HOLD_BUFFER_MAX as u64 + 1) {
        match interp.deliver(persist(seq, Some(&format!("b-{seq}")), seq + 1), 0) {
            Ok(()) => held += 1,
            Err(e) => failures.push(format!("hold of seq {seq} failed early: {e:?}")),
        }
    }
    if held != HOLD_BUFFER_MAX as u32 {
        failures.push(format!(
            "held {held}, want exactly HOLD_BUFFER_MAX={HOLD_BUFFER_MAX}"
        ));
    }
    match interp.deliver(persist(HOLD_BUFFER_MAX as u64 + 2, Some("b-over"), 99), 0) {
        Err(EffectError::OutOfOrder { seq, expected })
            if seq == HOLD_BUFFER_MAX as u64 + 2 && expected == 1 => {}
        other => failures.push(format!("past-bound delivery: wrong outcome: {other:?}")),
    }
    // The gap still fills afterwards: delivering seq 1 drains the
    // buffer in order.
    if let Err(e) = interp.deliver(persist(1, Some("b-1"), 2), 0) {
        failures.push(format!("gap-fill after bound failed: {e:?}"));
    }
    let seqs: Vec<u64> = interp.ledger.iter().map(|e| e.seq).collect();
    (failures, held, seqs)
}

/// A3: out-of-order deliveries park in the bounded hold buffer and
/// apply in seq order once the gap fills; past the bound the
/// interpreter rejects with `OutOfOrder{seq, expected}`.
fn case_out_of_order_bounded_hold() -> Result<CaseReport, TaskDriverError> {
    let mut failures = gap_fill_check();
    let (bound_failures, held, seqs2) = bound_overflow_check();
    failures.extend(bound_failures);
    let want: Vec<u64> = (0..=(HOLD_BUFFER_MAX as u64 + 1)).collect();
    if seqs2 != want {
        failures.push(format!("post-drain ledger {seqs2:?}, want {want:?}"));
    }
    let mut evidence = vec![
        format!(
            "gap fill: seqs [0, 2, 1] delivered -> ledger order {:?}: hold-then-drain works",
            [0, 1, 2]
        ),
        format!(
            "bound: {HOLD_BUFFER_MAX} held ok, seq {} refused with OutOfOrder{{expected: 1}}",
            HOLD_BUFFER_MAX as u64 + 2
        ),
        format!(
            "after the refusal the gap still fills: ledger is 0..={}",
            HOLD_BUFFER_MAX as u64 + 1
        ),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "hold_buffer_max": HOLD_BUFFER_MAX,
            "held": held,
            "refusal": "OutOfOrder",
            "ledger_ordered": seqs2 == want,
            "backend": "mock-interpreter-transport",
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
        "duplicate_delivery_idempotent" => case_duplicate_delivery_idempotent(),
        "keyless_retry_refused" => case_keyless_retry_refused(),
        "out_of_order_bounded_hold" => case_out_of_order_bounded_hold(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-169: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// duplicate-delivery idempotency itself.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-169".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-169".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
