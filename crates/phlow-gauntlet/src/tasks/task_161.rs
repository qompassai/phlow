// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 161 — snapshot resync after reconnect (rust, V).
//!
//! The client may have missed events during an outage; after reconnect
//! it takes a snapshot and converges to the daemon's current state
//! before processing live events. The driver pairs a scripted daemon
//! state ([`ScriptedDaemon`], MOCK) with the deterministic
//! [`ClientState`](crate::daemon_client::ClientState): V1 covers 5
//! missed events → snapshot → live resume with deep-equal state; V2
//! covers the snapshot racing live events — live frames are buffered
//! until the snapshot's `as_of` marker, then applied in order, with the
//! event ledger proving zero gaps and zero duplicates. A stale snapshot
//! replay is rejected with a typed error and leaves state untouched.

use std::collections::BTreeMap;

use crate::daemon_client::{ClientState, DaemonEvent, ResyncError, Snapshot};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-161";
/// Task name.
pub const NAME: &str = "snapshot resync after reconnect";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = [
    "missed_events_snapshot_resync",
    "snapshot_races_live_events",
];

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// Scripted daemon side (MOCK): an event log with a monotonic sequence
/// number and a state map; mints snapshots stamped with `as_of`.
struct ScriptedDaemon {
    version: u64,
    seq: u64,
    state: BTreeMap<String, String>,
}

impl ScriptedDaemon {
    fn new(version: u64) -> Self {
        Self {
            version,
            seq: 0,
            state: BTreeMap::new(),
        }
    }

    fn apply(&mut self, key: &str, value: &str) -> DaemonEvent {
        self.seq += 1;
        self.state.insert(key.to_string(), value.to_string());
        DaemonEvent {
            seq: self.seq,
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: self.version,
            as_of: self.seq,
            state: self.state.clone(),
        }
    }
}

/// Count ledger dispositions of one kind.
fn ledger_count(ledger: &[(u64, &'static str)], kind: &'static str) -> usize {
    ledger.iter().filter(|(_, k)| *k == kind).count()
}

/// V1: 5 events are missed during the outage. After reconnect the
/// client applies the snapshot, the live stream resumes, and the final
/// client state deep-equals the daemon state.
fn case_missed_events_snapshot_resync() -> Result<CaseReport, TaskDriverError> {
    let mut daemon = ScriptedDaemon::new(7);
    let mut client = ClientState::new();
    let mut failures = Vec::new();

    // The outage: 5 events the client never sees.
    for i in 1..=5u64 {
        daemon.apply(&format!("k{i}"), &format!("v{i}"));
    }
    // Reconnect: snapshot first, then live.
    client.begin_resync();
    client
        .on_snapshot(&daemon.snapshot())
        .map_err(|e| arm_error("snapshot", format!("snapshot rejected: {e:?}")))?;
    if client.state() != &daemon.state {
        failures.push("client state != daemon state after snapshot".to_string());
    }
    if client.applied_through() != 5 {
        failures.push(format!(
            "applied_through = {}, want 5",
            client.applied_through()
        ));
    }
    if client.version() != 7 {
        failures.push(format!("version = {}, want 7", client.version()));
    }
    // Live resumes: 2 more events, applied directly.
    for i in 6..=7u64 {
        let ev = daemon.apply(&format!("k{i}"), &format!("v{i}"));
        client
            .on_live_event(&ev)
            .map_err(|e| arm_error("live", format!("live event {i} rejected: {e:?}")))?;
    }
    if client.state() != &daemon.state {
        failures.push("client state != daemon state after live resume".to_string());
    }
    let ledger = client.ledger();
    if ledger_count(ledger, "dup-dropped") != 0 {
        failures.push(format!("ledger shows duplicates: {ledger:?}"));
    }
    let applied: Vec<u64> = ledger
        .iter()
        .filter(|(_, k)| *k == "applied")
        .map(|(s, _)| *s)
        .collect();
    if applied != [6, 7] {
        failures.push(format!("applied seqs {applied:?}, want [6, 7]"));
    }

    let evidence = vec![
        format!(
            "5 missed events → snapshot as_of=5 applied; applied_through = {}",
            client.applied_through()
        ),
        format!(
            "live resume: applied seqs {applied:?}; deep-equal = {}",
            client.state() == &daemon.state
        ),
        format!("ledger: {:?} (0 dup-dropped, 0 gaps)", ledger),
    ];
    finish(
        CASES[0],
        serde_json::json!({
            "applied_through": client.applied_through(),
            "version": client.version(),
            "deep_equal": client.state() == &daemon.state,
            "dup_dropped": ledger_count(ledger, "dup-dropped"),
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// V2: live events arrive *while* the snapshot is in flight. They are
/// buffered until the snapshot's `as_of` marker, then applied in
/// sequence order; a replayed live event is duplicate-dropped and a
/// stale snapshot is rejected with state untouched.
/// Build the final [`CaseReport`] from collected evidence and failures.
fn finish(
    case: &'static str,
    metrics: serde_json::Value,
    evidence: Vec<String>,
    failures: Vec<String>,
) -> Result<CaseReport, TaskDriverError> {
    let mut full_evidence = evidence;
    full_evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(case, metrics, full_evidence);
    report.failures = failures;
    report.passed = report.failures.is_empty();
    Ok(report)
}

/// Race phase: the snapshot is taken at seq 5, events 6 and 7 race it
/// in flight (buffered, not applied), then the snapshot lands and the
/// buffer drains in order. Returns the drain ledger.
fn race_and_drain(
    client: &mut ClientState,
    daemon: &mut ScriptedDaemon,
    failures: &mut Vec<String>,
) -> Result<Vec<(u64, &'static str)>, TaskDriverError> {
    for i in 1..=5u64 {
        daemon.apply(&format!("k{i}"), &format!("v{i}"));
    }
    // The snapshot is taken at seq 5; events 6 and 7 race it in flight.
    let snap = daemon.snapshot();
    client.begin_resync();
    for i in 6..=7u64 {
        let ev = daemon.apply(&format!("k{i}"), &format!("v{i}"));
        client
            .on_live_event(&ev)
            .map_err(|e| arm_error("race", format!("racing event {i} rejected: {e:?}")))?;
    }
    if !client.awaiting_snapshot() {
        failures.push("client left resync mode while snapshot in flight".to_string());
    }
    if client.applied_through() != 0 {
        failures.push("racing events were applied before the snapshot".to_string());
    }
    client
        .on_snapshot(&snap)
        .map_err(|e| arm_error("snapshot", format!("snapshot rejected: {e:?}")))?;
    let ledger = client.ledger().to_vec();
    // The ledger is append-only: the racing events are logged as
    // "buffered" when they arrive, then "applied" when the snapshot's
    // as_of marker lets the drain run.
    let want_ledger = vec![
        (6, "buffered"),
        (7, "buffered"),
        (5, "snapshot"),
        (6, "applied"),
        (7, "applied"),
    ];
    if ledger != want_ledger {
        failures.push(format!("ledger {ledger:?}, want {want_ledger:?}"));
    }
    Ok(ledger)
}

/// A stale snapshot (behind `applied_through`) is rejected with
/// [`ResyncError::StaleSnapshot`]; client state is untouched.
fn check_stale_rejection(
    client: &mut ClientState,
    daemon: &ScriptedDaemon,
    failures: &mut Vec<String>,
) {
    let state_before = client.state().clone();
    let stale = Snapshot {
        version: 7,
        as_of: 3,
        state: BTreeMap::new(),
    };
    match client.on_snapshot(&stale) {
        Err(ResyncError::StaleSnapshot {
            as_of,
            applied_through,
        }) => {
            if as_of != 3 || applied_through != 8 {
                failures.push(format!(
                    "StaleSnapshot carried ({as_of}, {applied_through}), want (3, 8)"
                ));
            }
        }
        other => failures.push(format!("stale snapshot gave {other:?}, want StaleSnapshot")),
    }
    if client.state() != &state_before {
        failures.push("stale snapshot mutated client state".to_string());
    }
    if client.state() != &daemon.state {
        failures.push("final client state != daemon state".to_string());
    }
}

fn case_snapshot_races_live_events() -> Result<CaseReport, TaskDriverError> {
    let mut daemon = ScriptedDaemon::new(7);
    let mut client = ClientState::new();
    let mut failures = Vec::new();

    let drain_ledger = race_and_drain(&mut client, &mut daemon, &mut failures)?;
    // Live resumes normally; a replayed event is a duplicate, not a gap.
    let ev8 = daemon.apply("k8", "v8");
    client
        .on_live_event(&ev8)
        .map_err(|e| arm_error("live", format!("live event 8 rejected: {e:?}")))?;
    let replay = DaemonEvent {
        seq: 6,
        key: "k6".to_string(),
        value: "v6".to_string(),
    };
    client
        .on_live_event(&replay)
        .map_err(|e| arm_error("replay", format!("replayed event rejected wrongly: {e:?}")))?;
    check_stale_rejection(&mut client, &daemon, &mut failures);
    let ledger = client.ledger();
    if ledger_count(ledger, "dup-dropped") != 1 {
        failures.push(format!(
            "want exactly 1 dup-dropped (the replay), ledger: {ledger:?}"
        ));
    }

    let evidence = vec![
        format!(
            "racing events 6,7 buffered until snapshot as_of=5; drain ledger: {drain_ledger:?}"
        ),
        "replayed event 6 → dup-dropped; stale snapshot as_of=3 → StaleSnapshot, state untouched"
            .to_string(),
        format!("final deep-equal = {}", client.state() == &daemon.state),
    ];
    finish(
        CASES[1],
        serde_json::json!({
            "race_ledger": ledger.iter().map(|(s, k)| serde_json::json!({"seq": s, "disposition": k})).collect::<Vec<_>>(),
            "dup_dropped": ledger_count(ledger, "dup-dropped"),
            "stale_rejected": true,
            "deep_equal": client.state() == &daemon.state,
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "missed_events_snapshot_resync" => case_missed_events_snapshot_resync(),
        "snapshot_races_live_events" => case_snapshot_races_live_events(),
        _ => Err(arm_error(
            "case",
            format!("task-161: unknown case '{case}'"),
        )),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-161".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-161".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
