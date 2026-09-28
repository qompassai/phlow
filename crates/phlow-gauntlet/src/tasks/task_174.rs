// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 174 — single-use code consumption (rust, A).
//!
//! A pairing code is a one-time ticket: after one successful verify
//! it is consumed, and any replay — same code, same secret — is
//! refused with [`PairingError::Consumed`]. Because the whole
//! verify sequence (lookup → checks → compare → consume → register)
//! runs under one mutex, two threads racing the same code produce
//! exactly one success and one `Consumed`: the replay cannot slip
//! through between the check and the consume.
//!
//! [`ManualClock`] (MOCK) drives time; threads share one `Arc<Daemon>`.
//! Single-use consumption adapts the consumed-flag discipline from
//! Ghostex `server/src/remote_access/pairing_code.rs`; the
//! atomicity argument and the race proof are ours.

use crate::bounty::clock::{Clock, ManualClock};
use crate::pairing::{Daemon, PairingError};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};
use std::sync::{Arc, Barrier};

/// Task id.
pub const ID: &str = "task-174";
/// Task name.
pub const NAME: &str = "single-use code consumption";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = ["replay_refused", "concurrent_verify_one_winner"];
/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 1_700_000_000;

fn daemon() -> Result<Daemon, TaskDriverError> {
    Daemon::new().map_err(|e| TaskDriverError::Fixture {
        what: "daemon".to_string(),
        detail: format!("{e:?}"),
    })
}

fn issue(
    daemon: &Daemon,
    label: &str,
    now: u64,
) -> Result<crate::pairing::IssuedCode, TaskDriverError> {
    daemon.issue(label, now).map_err(|e| TaskDriverError::Arm {
        arm: "issue".to_string(),
        detail: format!("{e:?}"),
    })
}

/// A1: a replayed code — same code, same correct secret, presented
/// again after a successful pairing — is refused with `Consumed`, and
/// no second device is registered.
fn case_replay_refused() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let daemon = daemon()?;
    let issued = issue(&daemon, "pixel-9", clock.now())?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    let mut first = issued.secret.clone();
    match daemon.verify(&issued.code, &mut first, clock.now()) {
        Ok(_) => evidence.push("first present: PairingOk".to_string()),
        Err(e) => failures.push(format!("first present failed: {e:?}, want PairingOk")),
    }
    let devices_after_first = daemon.device_count();
    // Replay: same code, same secret.
    let mut replay = issued.secret.clone();
    match daemon.verify(&issued.code, &mut replay, clock.now()) {
        Err(PairingError::Consumed) => {
            evidence.push("replay: Consumed".to_string());
        }
        other => failures.push(format!("replay: {other:?}, want Consumed")),
    }
    if daemon.device_count() != devices_after_first {
        failures.push(format!(
            "replay registered a device: count {} -> {}",
            devices_after_first,
            daemon.device_count()
        ));
    } else {
        evidence.push(format!(
            "device count unchanged at {}: replay registered nothing",
            devices_after_first
        ));
    }
    // The replayed secret buffer is zeroized too.
    if !replay.iter().all(|b| *b == 0) {
        failures.push("replay path left secret bytes in the caller buffer".to_string());
    } else {
        evidence.push("replay path: caller secret buffer zeroized".to_string());
    }

    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "replay_refused_as_consumed": true,
            "device_count": daemon.device_count(),
            "replay_secret_zeroized": true,
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// A2: two threads present the same code at the same instant (a
/// barrier starts them together). Exactly one wins with `Ok`; the
/// other gets `Consumed`. Exactly one device is registered.
fn case_concurrent_verify_one_winner() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let daemon = Arc::new(daemon()?);
    let issued = issue(&daemon, "pixel-9", clock.now())?;
    let mut failures = Vec::new();
    let now = clock.now();
    let barrier = Arc::new(Barrier::new(2));

    let mut handles = Vec::new();
    for racer in 0..2 {
        let daemon = Arc::clone(&daemon);
        let barrier = Arc::clone(&barrier);
        let code = issued.code.clone();
        let secret = issued.secret.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let mut secret = secret;
            let outcome = daemon.verify(&code, &mut secret, now);
            (racer, outcome)
        }));
    }
    let mut wins = 0;
    let mut consumed = 0;
    for handle in handles {
        match handle.join() {
            Ok((_, Ok(_))) => {
                wins += 1;
            }
            Ok((_, Err(PairingError::Consumed))) => {
                consumed += 1;
            }
            Ok((racer, other)) => failures.push(format!(
                "racer {racer}: {other:?}, want exactly one Ok and one Consumed"
            )),
            Err(_) => failures.push("a racer thread panicked".to_string()),
        }
    }
    if wins != 1 || consumed != 1 {
        failures.push(format!(
            "race outcome: {wins} wins, {consumed} Consumed — want 1 and 1"
        ));
    }
    if daemon.device_count() != 1 {
        failures.push(format!(
            "race registered {} devices, want exactly 1",
            daemon.device_count()
        ));
    }
    let evidence = vec![
        format!("concurrent verify: {wins} PairingOk, {consumed} Consumed"),
        format!("devices registered: {}", daemon.device_count()),
    ];

    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "wins": wins,
            "consumed": consumed,
            "device_count": daemon.device_count(),
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
        "replay_refused" => case_replay_refused(),
        "concurrent_verify_one_winner" => case_concurrent_verify_one_winner(),
        _ => Err(TaskDriverError::Arm {
            arm: "case".to_string(),
            detail: format!("task-174: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-174".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-174".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
