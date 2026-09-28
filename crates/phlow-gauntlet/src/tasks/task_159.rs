// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 159 — reconnect ladder (rust, V).
//!
//! The worker thread must reconnect on an escalating ladder, not a hot
//! loop: attempts are bounded and spaced, and the ladder resets only
//! after a genuinely healthy stream. The driver drives the deterministic
//! [`ReconnectEngine`](crate::daemon_client::ReconnectEngine) against a
//! scripted [`ManualClock`] plus a [`ScriptedLink`](crate::daemon_client::ScriptedLink)
//! double (MOCK) whose connect outcomes are scripted: drop, drop, drop,
//! accept (V1) or never accept (V2).
//!
//! Time note: [`ManualClock`] resolves whole seconds while the ladder
//! resolves milliseconds, so the driver keeps an ms scripted timeline
//! and mirrors it into the clock (`clock.set(ms / 1000)`). The clock
//! stays the single scripted time authority — nothing reads wall time —
//! and the ms ledger lets attempt timestamps be asserted exactly
//! (deviation 0, within the one-tick bar).

use crate::bounty::clock::{Clock, ManualClock};
use crate::daemon_client::{
    HEALTHY_STREAM_MS, Link, MAX_RECONNECT_ATTEMPTS, ParkReason, RECONNECT_LADDER_MS,
    ReconnectEngine, ScriptedLink, ScriptedRead,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-159";
/// Task name.
pub const NAME: &str = "reconnect ladder";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["reconnect_ladder_three_drops", "backoff_exhausted_parks"];
/// Scripted timeline start (ms). Mirrored into the [`ManualClock`] at
/// second resolution.
const T0_MS: u64 = 1_000_000_000;

/// Scripted time: an ms timeline mirrored into the [`ManualClock`].
struct ScriptedTime {
    now_ms: u64,
    clock: ManualClock,
}

impl ScriptedTime {
    fn new() -> Self {
        Self {
            now_ms: T0_MS,
            clock: ManualClock::new(T0_MS / 1000),
        }
    }

    /// Jump to an absolute ms timestamp.
    fn set_ms(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        self.clock.set(now_ms / 1000);
    }

    fn clock_secs(&self) -> u64 {
        self.clock.now()
    }
}

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// Drive `drops` accept-then-drop cycles on the engine's absolute-ms
/// timeline, returning the next scheduled attempt time. The engine's
/// initial deadline (Some(0)) means "now" on the driver's timeline, so
/// the first attempt goes at T0 and every deadline the engine returns
/// is used verbatim — never re-based onto the epoch.
fn drive_drops(
    engine: &mut ReconnectEngine,
    link: &mut ScriptedLink,
    time: &mut ScriptedTime,
    failures: &mut Vec<String>,
    drops: u32,
) -> Result<u64, TaskDriverError> {
    let mut now_ms = T0_MS;
    for drop in 1..=drops {
        time.set_ms(now_ms);
        if !engine.note_attempt(now_ms) {
            failures.push(format!("attempt {drop} refused while unparked"));
        }
        if link.connect().is_ok() {
            failures.push(format!("drop {drop}: daemon accepted, want a drop"));
        }
        engine.note_disconnect(now_ms, false);
        now_ms = engine.next_attempt_at_ms().ok_or_else(|| {
            arm_error(
                "ladder",
                format!("parked before drop {drop}, want {drops} first"),
            )
        })?;
    }
    Ok(now_ms)
}

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

/// V1: the daemon drops the connection 3 times, then accepts. Attempt
/// timestamps must be exactly T0, T0+100, T0+600, T0+2600 — the ladder
/// [100, 500, 2000] cumulative. The 4th attempt connects, the stream
/// stays healthy past [`HEALTHY_STREAM_MS`] and closes orderly, and the
/// ladder resets: the next failure schedules the floor rung again.
fn case_reconnect_ladder_three_drops() -> Result<CaseReport, TaskDriverError> {
    let mut time = ScriptedTime::new();
    let mut engine = ReconnectEngine::new();
    let mut link = ScriptedLink::new(false, ScriptedRead::Timeout, true);
    link.script_connects(&[false, false, false, true]);
    let mut failures = Vec::new();

    let now_ms = drive_drops(&mut engine, &mut link, &mut time, &mut failures, 3)?;
    // Fourth attempt: the daemon accepts.
    time.set_ms(now_ms);
    if !engine.note_attempt(now_ms) {
        failures.push("fourth attempt refused while unparked".to_string());
    }
    if let Err(e) = link.connect() {
        failures.push(format!("fourth attempt failed to connect: {e:?}"));
    }
    link.close();
    let attempt_times: Vec<u64> = engine.attempt_times_ms().to_vec();
    let want_times = [T0_MS, T0_MS + 100, T0_MS + 600, T0_MS + 2600];
    if attempt_times != want_times {
        failures.push(format!(
            "attempt timestamps {attempt_times:?}, want ladder-cumulative {want_times:?}"
        ));
    }
    if engine.consecutive_failures() != 3 {
        failures.push(format!(
            "consecutive failures {}, want 3 after three drops",
            engine.consecutive_failures()
        ));
    }
    // Healthy stream, orderly close: the ladder must reset.
    let healthy_close_at = now_ms + HEALTHY_STREAM_MS;
    time.set_ms(healthy_close_at);
    engine.note_disconnect(healthy_close_at, true);
    check_reset(&engine, healthy_close_at, &mut failures)?;

    let evidence = vec![
        format!("attempt timestamps (ms): {attempt_times:?}"),
        format!("ladder cumulative want: {want_times:?}; deviation 0 (bar: within one 1s tick)"),
        format!(
            "after healthy ({} ms) + orderly close: consecutive_failures = {}, next delay = {} ms (floor)",
            HEALTHY_STREAM_MS,
            engine.consecutive_failures(),
            RECONNECT_LADDER_MS[0]
        ),
        format!("scripted clock mirrored at {} s", time.clock_secs()),
    ];
    finish(
        CASES[0],
        serde_json::json!({
            "attempt_times_ms": attempt_times,
            "ladder_delays_ms": [100, 500, 2000],
            "reset_observed": engine.consecutive_failures() == 0,
            "parked": engine.parked().is_some(),
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// Assert the ladder reset after a healthy + orderly close: the counter
/// is 0 and the next failure schedules the floor rung.
fn check_reset(
    engine: &ReconnectEngine,
    healthy_close_at: u64,
    failures: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    if engine.consecutive_failures() != 0 {
        failures.push(format!(
            "ladder did not reset after healthy+orderly close: failures = {}",
            engine.consecutive_failures()
        ));
    }
    let reset_at = engine.next_attempt_at_ms().ok_or_else(|| {
        arm_error(
            "ladder",
            "no attempt scheduled after healthy close".to_string(),
        )
    })?;
    if reset_at != healthy_close_at + RECONNECT_LADDER_MS[0] {
        failures.push(format!(
            "post-reset delay {}, want floor rung {}",
            reset_at - healthy_close_at,
            RECONNECT_LADDER_MS[0]
        ));
    }
    Ok(())
}

/// Attempt until the engine parks, returning the attempt count. The
/// daemon is down for the whole run, so every attempt fails.
fn drive_until_parked(
    engine: &mut ReconnectEngine,
    link: &mut ScriptedLink,
    time: &mut ScriptedTime,
    failures: &mut Vec<String>,
) -> u32 {
    let mut attempts = 0u32;
    let mut now_ms = T0_MS;
    loop {
        time.set_ms(now_ms);
        if !engine.note_attempt(now_ms) {
            break; // parked: the driver must not attempt
        }
        if link.connect().is_ok() {
            failures.push(format!(
                "attempt {}: daemon accepted, want down",
                attempts + 1
            ));
        }
        engine.note_disconnect(now_ms, false);
        attempts += 1;
        match engine.next_attempt_at_ms() {
            Some(at) => now_ms = at,
            None => break, // parked
        }
    }
    attempts
}

/// Parked window: 60 s of [`ManualClock`] time. The engine has no
/// deadline (`next_attempt_at_ms()` is `None`), so a deadline-driven
/// waiter performs zero wakeups — the no-busy-loop bar.
fn count_parked_wakeups(
    engine: &ReconnectEngine,
    time: &mut ScriptedTime,
    failures: &mut Vec<String>,
) -> u32 {
    time.clock.advance(60);
    let mut wakeups = 0u32;
    while engine.next_attempt_at_ms().is_some() {
        wakeups += 1;
        if wakeups > 1000 {
            failures.push("parked engine still scheduling attempts".to_string());
            break;
        }
    }
    if wakeups != 0 {
        failures.push(format!("{wakeups} wakeups while parked, want 0"));
    }
    wakeups
}

/// V2: the daemon is down for the whole test. Attempts stop at
/// [`MAX_RECONNECT_ATTEMPTS`], the worker parks in `BackoffExhausted`,
/// and over 60 s of further [`ManualClock`] time the parked engine
/// performs zero wakeups — a deadline-driven wait, not a spin.
fn case_backoff_exhausted_parks() -> Result<CaseReport, TaskDriverError> {
    let mut time = ScriptedTime::new();
    let mut engine = ReconnectEngine::new();
    let mut link = ScriptedLink::new(false, ScriptedRead::Timeout, true);
    let mut failures = Vec::new();

    let attempts = drive_until_parked(&mut engine, &mut link, &mut time, &mut failures);
    if attempts != MAX_RECONNECT_ATTEMPTS {
        failures.push(format!(
            "attempts {attempts}, want exactly MAX_RECONNECT_ATTEMPTS ({MAX_RECONNECT_ATTEMPTS})"
        ));
    }
    if engine.parked() != Some(ParkReason::BackoffExhausted) {
        failures.push(format!(
            "parked = {:?}, want BackoffExhausted",
            engine.parked()
        ));
    }
    let times = engine.attempt_times_ms().to_vec();
    let span_ms = times.last().copied().unwrap_or(0) - times.first().copied().unwrap_or(0);
    // 9 gaps: ladder rungs 1..=9 = 100+500+2000+4000+8000+16000*4.
    let want_span = 100 + 500 + 2000 + 4000 + 8000 + 16000 * 4;
    if span_ms != want_span {
        failures.push(format!("attempt span {span_ms} ms, want {want_span} ms"));
    }
    let wakeups = count_parked_wakeups(&engine, &mut time, &mut failures);
    if engine.attempt_times_ms().len() != times.len() {
        failures.push("attempt ledger grew while parked".to_string());
    }

    let evidence = vec![
        format!("{attempts} attempts then parked: {:?}", engine.parked()),
        format!("attempt span {span_ms} ms == ladder sum {want_span} ms"),
        format!("parked 60 s of ManualClock time: {wakeups} wakeups, ledger unchanged"),
    ];
    finish(
        CASES[1],
        serde_json::json!({
            "attempts": attempts,
            "max_reconnect_attempts": MAX_RECONNECT_ATTEMPTS,
            "parked": format!("{:?}", engine.parked()),
            "attempt_span_ms": span_ms,
            "parked_window_wakeups": wakeups,
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "reconnect_ladder_three_drops" => case_reconnect_ladder_three_drops(),
        "backoff_exhausted_parks" => case_backoff_exhausted_parks(),
        _ => Err(arm_error(
            "case",
            format!("task-159: unknown case '{case}'"),
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
            where_: "task-159".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-159".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
