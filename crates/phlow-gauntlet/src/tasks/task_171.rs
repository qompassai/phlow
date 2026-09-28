// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 171 — pairing code TTL expiry (rust, V).
//!
//! A code older than 15 minutes is dead: presented at T+899 s it is
//! accepted, at exactly the TTL second (T+900 s) and after it is
//! refused with [`PairingError::Expired`]. Dead codes do not
//! accumulate — the purge job drops every record with
//! `expires_at < now`, and the driver asserts the purge of 1,000
//! expired codes completes inside a named time bound.
//!
//! [`ManualClock`] (MOCK) drives all times; the store is in-memory.
//! The 15-minute TTL adapts Ghostex `REMOTE_PAIRING_SECRET_TTL`; the
//! exact-second boundary and the purge cadence are ours.

use crate::bounty::clock::{Clock, ManualClock};
use crate::pairing::{Daemon, PairingError, TTL_SECS};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-171";
/// Task name.
pub const NAME: &str = "pairing code TTL expiry";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["ttl_boundary", "purge_expired_bounded"];
/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 1_700_000_000;
/// Purge of 1,000 expired codes must finish inside this bound.
pub const PURGE_TIME_BOUND_SECS: u64 = 5;
/// Expired codes planted for the purge case.
pub const EXPIRED_COUNT: usize = 1000;
/// Live codes that must survive the purge.
pub const LIVE_COUNT: usize = 3;

fn daemon() -> Result<Daemon, TaskDriverError> {
    Daemon::new().map_err(|e| TaskDriverError::Fixture {
        what: "daemon".to_string(),
        detail: format!("{e:?}"),
    })
}

/// V1: the boundary is exact at the TTL second. T+899 s → accepted;
/// T+900 s (elapsed == TTL_SECS) and T+901 s → `Expired`.
fn case_ttl_boundary() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    // Fresh code per probe: a successful verify consumes the code,
    // so each boundary point needs its own issuance.
    for (probe_at, want_ok) in [(899u64, true), (900, false), (901, false)] {
        let mut clock = ManualClock::new(CLOCK_START);
        let daemon = daemon()?;
        let issued = daemon
            .issue("pixel-9", clock.now())
            .map_err(|e| TaskDriverError::Arm {
                arm: "issue".to_string(),
                detail: format!("{e:?}"),
            })?;
        clock.advance(probe_at);
        let mut secret = issued.secret.clone();
        let outcome = daemon.verify(&issued.code, &mut secret, clock.now());
        let ok = outcome.is_ok();
        if ok != want_ok {
            failures.push(format!(
                "present at T+{probe_at}s: {outcome:?}, want {}",
                if want_ok { "PairingOk" } else { "Expired" }
            ));
        } else if !want_ok {
            match outcome {
                Err(PairingError::Expired) => {
                    evidence.push(format!("T+{probe_at}s: Expired as required"));
                }
                other => failures.push(format!(
                    "T+{probe_at}s refused but as {other:?}, want Expired"
                )),
            }
        } else {
            evidence.push(format!("T+{probe_at}s: PairingOk (inside TTL)"));
        }
    }

    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "accepted_at_899": true,
            "expired_at_900": true,
            "expired_at_901": true,
            "ttl_secs": TTL_SECS,
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// V2: 1,000 expired codes plus 3 live ones; the purge removes
/// exactly the expired set, keeps the live set, and finishes inside
/// `PURGE_TIME_BOUND_SECS`.
/// Seed `EXPIRED_COUNT` codes, age the store past the TTL, then
/// plant `LIVE_COUNT` live codes at the new now — so the purge has
/// something to remove and something to keep.
fn seed_mixed_store(clock: &mut ManualClock, daemon: &Daemon) -> Result<(), TaskDriverError> {
    for i in 0..EXPIRED_COUNT {
        daemon
            .issue(&format!("expired-{i}"), clock.now())
            .map_err(|e| TaskDriverError::Arm {
                arm: "issue".to_string(),
                detail: format!("{e:?}"),
            })?;
    }
    clock.advance(TTL_SECS + 1);
    for i in 0..LIVE_COUNT {
        daemon
            .issue(&format!("live-{i}"), clock.now())
            .map_err(|e| TaskDriverError::Arm {
                arm: "issue".to_string(),
                detail: format!("{e:?}"),
            })?;
    }
    Ok(())
}

/// Run the purge under a wall-clock bound and assert the counts:
/// exactly the expired codes are removed, the live ones remain.
fn run_timed_purge(
    daemon: &Daemon,
    clock_now: u64,
    failures: &mut Vec<String>,
) -> (usize, usize, u128) {
    let started = std::time::Instant::now();
    let purged = daemon.purge_expired(clock_now);
    let purge_ms = started.elapsed().as_millis();
    let remaining_after_purge = daemon.code_count();
    if purged != EXPIRED_COUNT {
        failures.push(format!("purged {purged} codes, want {EXPIRED_COUNT}"));
    }
    if remaining_after_purge != LIVE_COUNT {
        failures.push(format!(
            "store holds {remaining_after_purge} codes after purge, want {LIVE_COUNT} live"
        ));
    }
    let bound_ms = u128::from(PURGE_TIME_BOUND_SECS) * 1000;
    if purge_ms > bound_ms {
        failures.push(format!("purge took {purge_ms}ms, bound is {bound_ms}ms"));
    }
    (purged, remaining_after_purge, purge_ms)
}

/// The survivors still pair: freshly issued codes verify cleanly
/// after the purge ran.
fn probe_survivor_pairing(daemon: &Daemon, clock_now: u64, failures: &mut Vec<String>) -> bool {
    let survivor_ok = (0..LIVE_COUNT).all(|_| {
        daemon
            .issue("survivor-probe", clock_now)
            .and_then(|issued| {
                let mut secret = issued.secret.clone();
                daemon.verify(&issued.code, &mut secret, clock_now)
            })
            .is_ok()
    });
    if !survivor_ok {
        failures.push("a freshly issued code failed to pair after the purge".to_string());
    }
    survivor_ok
}

fn case_purge_expired_bounded() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(CLOCK_START);
    let daemon = daemon()?;
    seed_mixed_store(&mut clock, &daemon)?;
    let mut failures = Vec::new();
    if daemon.code_count() != EXPIRED_COUNT + LIVE_COUNT {
        failures.push(format!(
            "store holds {} codes, want {}",
            daemon.code_count(),
            EXPIRED_COUNT + LIVE_COUNT
        ));
    }
    let (purged, remaining_after_purge, purge_ms) =
        run_timed_purge(&daemon, clock.now(), &mut failures);
    let bound_ms = u128::from(PURGE_TIME_BOUND_SECS) * 1000;
    let survivor_ok = probe_survivor_pairing(&daemon, clock.now(), &mut failures);
    let evidence = vec![
        format!(
            "purge: removed {purged}/{EXPIRED_COUNT} expired, \
             {LIVE_COUNT} live remain, took {purge_ms}ms (bound {bound_ms}ms)"
        ),
        format!("post-purge pairing works: {survivor_ok}"),
    ];

    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "purged": purged,
            "remaining": remaining_after_purge,
            "purge_ms": purge_ms,
            "purge_bound_ms": bound_ms,
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
        "ttl_boundary" => case_ttl_boundary(),
        "purge_expired_bounded" => case_purge_expired_bounded(),
        _ => Err(TaskDriverError::Arm {
            arm: "case".to_string(),
            detail: format!("task-171: unknown case '{case}'"),
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
            where_: "task-171".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-171".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
