// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 176 — sidecar supervision under failure (rust, V).
//!
//! The daemon supervises its sidecar the way Ghostex's `tailcat`
//! supervisor watches the tunnel helper: a child that exits is
//! restarted, a child that keeps dying is parked after a bounded
//! number of restarts — and the daemon's own loopback service never
//! goes down with it. Two scripted fixture binaries drive the cases:
//! one that exits immediately (restart bound) and one that answers
//! PING with PONG until it is SIGKILLed (mid-pairing death).
//!
//! The remote pairing path (`verify_remote`) gates on a live sidecar
//! and a successful PING/PONG round-trip. SIGKILLing the child while
//! the round-trip is in flight fails typed as
//! [`PairingError::SidecarDown`] inside
//! [`crate::pairing::TUNNEL_ROUNDTRIP_TIMEOUT`] — the pump thread's
//! EOF and the bounded `recv_timeout` mean the caller can never
//! hang.
//!
//! [`ManualClock`] (MOCK) drives pairing time. The fixtures are
//! POSIX shell scripts; the supervision discipline adapts Ghostex
//! `server/src/tailcat/supervisor.rs`, the PING/PONG tunnel gate and
//! the poll-step design are ours.

use crate::bounty::clock::{Clock, ManualClock};
use crate::pairing::{
    Daemon, IssuedCode, MAX_SIDECAR_RESTARTS, PairingError, SidecarSpec, SidecarStatus,
    TUNNEL_ROUNDTRIP_TIMEOUT,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Task id.
pub const ID: &str = "task-176";
/// Task name.
pub const NAME: &str = "sidecar supervision under failure";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["crash_loop_parks_bounded", "sigkill_mid_pairing_typed"];
/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 1_700_000_000;
/// The remote-verify result must arrive inside this bound, proving
/// the mid-pairing death failed instead of hanging.
pub const REMOTE_RESULT_BOUND: std::time::Duration = std::time::Duration::from_secs(10);

fn daemon() -> Result<Daemon, TaskDriverError> {
    Daemon::new().map_err(|e| TaskDriverError::Fixture {
        what: "daemon".to_string(),
        detail: format!("{e:?}"),
    })
}

/// Write a fixture shell script under a fresh temp dir and return its
/// path. `sh` and `sleep` come from the runner's PATH — POSIX only.
fn fixture_script(name: &str, body: &str) -> Result<PathBuf, TaskDriverError> {
    let dir = std::env::temp_dir().join(format!("phlow-gauntlet-176-{name}"));
    std::fs::create_dir_all(&dir).map_err(|e| TaskDriverError::Fixture {
        what: "fixture dir".to_string(),
        detail: e.to_string(),
    })?;
    let path = dir.join(format!("{name}.sh"));
    std::fs::write(&path, body).map_err(|e| TaskDriverError::Fixture {
        what: "fixture script".to_string(),
        detail: e.to_string(),
    })?;
    Ok(path)
}

/// V1: the sidecar is a script that exits 1 at once. Each poll reaps
/// and restarts it until `MAX_SIDECAR_RESTARTS` restarts, then the
/// supervisor parks it in `Failed` and stops spawning. The daemon's
/// loopback path still pairs a device the whole time.
/// Drive the supervisor until it parks, bounded so a runaway
/// supervisor cannot spin the caller forever. The settle sleep
/// matters: the crasher exits ~1 ms after spawn, so a tight loop
/// would burn the poll budget on "still alive" observations before
/// the first exit is ever seen. Returns (polls, restarts).
fn drive_until_parked(daemon: &Daemon, failures: &mut Vec<String>) -> (u32, u32) {
    let mut polls = 0u32;
    while daemon.sidecar_status()
        != Some(SidecarStatus::Failed {
            restarts: MAX_SIDECAR_RESTARTS,
        })
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
        daemon.sidecar_poll();
        polls += 1;
        if polls > MAX_SIDECAR_RESTARTS + 10 {
            failures.push(format!(
                "supervisor did not park after {polls} polls; status={:?}",
                daemon.sidecar_status()
            ));
            break;
        }
    }
    (polls, daemon.sidecar_restart_count())
}

/// Parked means parked: further polls spawn nothing.
fn check_parked_stable(daemon: &Daemon, failures: &mut Vec<String>) {
    let restarts_before = daemon.sidecar_restart_count();
    for _ in 0..5 {
        daemon.sidecar_poll();
    }
    if daemon.sidecar_restart_count() != restarts_before {
        failures.push("parked supervisor spawned the sidecar again".to_string());
    }
}

/// The daemon itself never went down: loopback pairing works after
/// the sidecar parked.
fn prove_daemon_unaffected(
    daemon: &Daemon,
    clock_now: u64,
    failures: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let issued = daemon
        .issue("pixel-9", clock_now)
        .map_err(|e| TaskDriverError::Arm {
            arm: "issue".to_string(),
            detail: format!("{e:?}"),
        })?;
    let mut secret = issued.secret.clone();
    match daemon.verify(&issued.code, &mut secret, clock_now) {
        Ok(_) => {}
        Err(e) => failures.push(format!("loopback verify after park: {e:?}, want Ok")),
    }
    Ok(())
}

fn case_crash_loop_parks_bounded() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let daemon = daemon()?;
    let script = fixture_script("crasher", "#!/bin/sh\nexit 1\n")?;
    daemon.attach_sidecar(SidecarSpec {
        bin: PathBuf::from("sh"),
        args: vec![script.to_string_lossy().into_owned()],
    });
    let mut failures = Vec::new();

    let (polls, restarts) = drive_until_parked(&daemon, &mut failures);
    if restarts != MAX_SIDECAR_RESTARTS {
        failures.push(format!(
            "restart count {restarts}, want exactly {MAX_SIDECAR_RESTARTS}"
        ));
    }
    check_parked_stable(&daemon, &mut failures);
    prove_daemon_unaffected(&daemon, clock.now(), &mut failures)?;
    let evidence = vec![
        format!(
            "crasher sidecar: parked after {restarts} restarts over {polls} polls; \
             status={:?}",
            daemon.sidecar_status()
        ),
        "loopback pairing after park: PairingOk (daemon unaffected)".to_string(),
    ];

    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "restarts": restarts,
            "restart_bound": MAX_SIDECAR_RESTARTS,
            "polls": polls,
            "status": format!("{:?}", daemon.sidecar_status()),
            "loopback_ok": true,
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Kill mid-flight: the verify thread blocks in the tunnel
/// round-trip (the stalled ponger never answers); the killer
/// SIGKILLs the child 200 ms in, and the pump thread's EOF surfaces
/// as `SidecarDown` — typed, inside the no-hang bound.
fn kill_mid_flight(
    daemon: &std::sync::Arc<Daemon>,
    issued: &IssuedCode,
    clock_now: u64,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) {
    let daemon_for_verify = std::sync::Arc::clone(daemon);
    let code = issued.code.clone();
    let secret = issued.secret.clone();
    let verify_thread = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut secret = secret;
        let outcome = daemon_for_verify.verify_remote(&code, &mut secret, clock_now);
        (outcome, started.elapsed())
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    daemon.sidecar_kill_child();
    match verify_thread.join() {
        Ok((Err(PairingError::SidecarDown), elapsed)) => {
            evidence.push(format!(
                "SIGKILL mid-pairing: SidecarDown after {}ms (bound {:?})",
                elapsed.as_millis(),
                REMOTE_RESULT_BOUND
            ));
            if elapsed > REMOTE_RESULT_BOUND {
                failures.push(format!(
                    "typed failure took {:?}, exceeding the no-hang bound {:?}",
                    elapsed, REMOTE_RESULT_BOUND
                ));
            }
        }
        Ok((other, elapsed)) => failures.push(format!(
            "SIGKILL mid-pairing: {other:?} after {:?}, want SidecarDown",
            elapsed
        )),
        Err(_) => failures.push("verify thread panicked".to_string()),
    }
}

/// The daemon is still up after the kill: loopback pairing works,
/// and the supervisor can still observe the dead child on poll.
fn prove_daemon_survived(
    daemon: &Daemon,
    clock_now: u64,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let loopback =
        daemon
            .issue("pixel-9-loopback", clock_now)
            .map_err(|e| TaskDriverError::Arm {
                arm: "issue".to_string(),
                detail: format!("{e:?}"),
            })?;
    let mut loopback_secret = loopback.secret.clone();
    match daemon.verify(&loopback.code, &mut loopback_secret, clock_now) {
        Ok(_) => evidence.push("loopback after SIGKILL: PairingOk".to_string()),
        Err(e) => failures.push(format!("loopback after SIGKILL: {e:?}, want Ok")),
    }
    daemon.sidecar_poll();
    evidence.push(format!(
        "supervisor observed the kill: status={:?}",
        daemon.sidecar_status()
    ));
    daemon.sidecar_stop();
    Ok(())
}

/// Sanity: the remote path works while the sidecar is alive —
/// the fast ponger answers PING with PONG.
fn prove_remote_alive(
    daemon: &Daemon,
    clock_now: u64,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let warm = daemon
        .issue("warm", clock_now)
        .map_err(|e| TaskDriverError::Arm {
            arm: "issue".to_string(),
            detail: format!("{e:?}"),
        })?;
    let mut warm_secret = warm.secret.clone();
    match daemon.verify_remote(&warm.code, &mut warm_secret, clock_now) {
        Ok(_) => evidence.push("remote verify while alive: PairingOk".to_string()),
        Err(e) => failures.push(format!("remote verify while alive: {e:?}, want Ok")),
    }
    Ok(())
}

/// V2: the sidecar answers PING with PONG, so the remote path is
/// live. A thread starts `verify_remote`; once it is inside the
/// round-trip, the test SIGKILLs the child. The verify must return
/// `SidecarDown` — typed, not a hang — inside `REMOTE_RESULT_BOUND`,
/// and the daemon plus loopback pairing must still be up.
///
/// Two fixtures: a fast ponger proves the remote path works while the
/// sidecar is alive, then a stalled ponger — it reads the PING, then
/// blocks on a *second* read that never comes, spawning no
/// subprocess — keeps the round-trip in flight for the SIGKILL to
/// land mid-way. (A `sleep 30` fixture would orphan the `sleep` when
/// the shell is SIGKILLed and hold the pipe open; the two-read
/// blocker dies with the shell, so stdout closes and the pump
/// thread's EOF surfaces promptly.)
fn case_sigkill_mid_pairing_typed() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let daemon = std::sync::Arc::new(daemon()?);
    let fast = fixture_script(
        "ponger-fast",
        "#!/bin/sh\nwhile IFS= read -r _; do echo PONG; done\n",
    )?;
    daemon.attach_sidecar(SidecarSpec {
        bin: PathBuf::from("sh"),
        args: vec![fast.to_string_lossy().into_owned()],
    });
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    prove_remote_alive(&daemon, clock.now(), &mut failures, &mut evidence)?;

    // Swap in the stalled ponger: attaching drops the old
    // supervisor, whose Drop kills and reaps the fast child. The
    // stalled ponger reads the PING then blocks on a second read
    // that never comes — no subprocess, no PONG — so the SIGKILL
    // closes stdout and the pump thread's EOF surfaces promptly.
    let stalled = fixture_script(
        "ponger-stalled",
        "#!/bin/sh\nwhile IFS= read -r _; do IFS= read -r _; echo PONG; done\n",
    )?;
    daemon.attach_sidecar(SidecarSpec {
        bin: PathBuf::from("sh"),
        args: vec![stalled.to_string_lossy().into_owned()],
    });

    let issued = daemon
        .issue("pixel-9", clock.now())
        .map_err(|e| TaskDriverError::Arm {
            arm: "issue".to_string(),
            detail: format!("{e:?}"),
        })?;
    kill_mid_flight(&daemon, &issued, clock.now(), &mut failures, &mut evidence);
    prove_daemon_survived(&daemon, clock.now(), &mut failures, &mut evidence)?;

    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "mid_pairing_failure": "SidecarDown",
            "tunnel_roundtrip_timeout_ms": TUNNEL_ROUNDTRIP_TIMEOUT.as_millis(),
            "no_hang_bound_ms": REMOTE_RESULT_BOUND.as_millis(),
            "loopback_ok": true,
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
        "crash_loop_parks_bounded" => case_crash_loop_parks_bounded(),
        "sigkill_mid_pairing_typed" => case_sigkill_mid_pairing_typed(),
        _ => Err(TaskDriverError::Arm {
            arm: "case".to_string(),
            detail: format!("task-176: unknown case '{case}'"),
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
            where_: "task-176".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-176".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
