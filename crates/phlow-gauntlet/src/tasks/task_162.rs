// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 162 — flapping daemon backoff (rust, A).
//!
//! A daemon (or network) that flaps rapidly must not turn the client
//! into a busy loop or a fork bomb of connection attempts. A1 drives
//! 1,000 accept-then-drop flaps at the deterministic
//! [`ReconnectEngine`](crate::daemon_client::ReconnectEngine): attempts
//! stay capped, inter-attempt intervals never drop below the ladder
//! floor, and the scripted link's handle census (plus the real fd count)
//! proves every accepted socket was released exactly once. A2 is the
//! nastier variant: the adversary syncs each abrupt drop to land exactly
//! on the ladder-reset boundary, trying to collapse the backoff to the
//! minimum interval. The engine's consecutive-failure counter survives —
//! only an orderly close after a healthy stream earns a reset (the
//! deliberate deviation from Ghostex documented in
//! [`crate::daemon_client`]).

use crate::bounty::clock::ManualClock;
use crate::daemon_client::{
    HEALTHY_STREAM_MS, Link, LinkError, MAX_RECONNECT_ATTEMPTS, ParkReason, RECONNECT_LADDER_MS,
    ReconnectEngine, ScriptedLink, ScriptedRead, fd_count,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-162";
/// Task name.
pub const NAME: &str = "flapping daemon backoff";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = ["thousand_flaps_capped", "synced_flap_no_collapse"];
/// Scripted timeline start (ms); see task-159 for the mirroring note.
const T0_MS: u64 = 2_000_000_000;

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// One flap: the daemon accepts, then immediately drops. The worker
/// releases the socket exactly once per accepted connection (mirroring
/// the close-after-leave in the adapted worker loop).
fn flap(
    engine: &mut ReconnectEngine,
    link: &mut ScriptedLink,
    now_ms: u64,
) -> Result<bool, TaskDriverError> {
    if !engine.note_attempt(now_ms) {
        return Ok(false);
    }
    link.connect()
        .map_err(|e| arm_error("flap", format!("daemon refused the accept: {e:?}")))?;
    match link.read_frame(std::time::Duration::from_millis(1)) {
        Err(LinkError::Dropped) => {}
        other => {
            return Err(arm_error(
                "flap",
                format!("scripted drop gave {other:?}, want Dropped"),
            ));
        }
    }
    link.close();
    engine.note_disconnect(now_ms, false);
    Ok(true)
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

/// Drive up to 1,000 accept-then-drop flaps on the engine's absolute-ms
/// timeline, returning the client attempt count. The engine's initial
/// deadline (Some(0)) means "now" on the driver's timeline, so the
/// first flap goes at T0 and every deadline it returns is used verbatim.
fn drive_flaps(
    engine: &mut ReconnectEngine,
    link: &mut ScriptedLink,
    clock: &mut ManualClock,
    failures: &mut Vec<String>,
) -> Result<u32, TaskDriverError> {
    let mut flaps = 0u32;
    let mut now_ms = T0_MS;
    for _ in 0..1000u32 {
        clock.set(now_ms / 1000);
        if !flap(engine, link, now_ms)? {
            break; // parked: the remaining flaps find no client
        }
        flaps += 1;
        match engine.next_attempt_at_ms() {
            Some(at) => now_ms = at,
            None => break, // parked
        }
    }
    if flaps != MAX_RECONNECT_ATTEMPTS {
        failures.push(format!(
            "client attempts {flaps}, want exactly MAX_RECONNECT_ATTEMPTS ({MAX_RECONNECT_ATTEMPTS})"
        ));
    }
    Ok(flaps)
}

/// The link census: every accepted socket released exactly once, no
/// open handles, fd count back to baseline.
fn check_census(link: &ScriptedLink, fds_before: usize, failures: &mut Vec<String>) {
    if link.connect_count() != link.close_count() {
        failures.push(format!(
            "connects={} closes={}: every accepted socket must be released exactly once",
            link.connect_count(),
            link.close_count()
        ));
    }
    if link.open_handles() != 0 {
        failures.push(format!("{} link handles still open", link.open_handles()));
    }
    let fds_after = fd_count();
    if fds_after != fds_before {
        failures.push(format!(
            "fd count {fds_before} → {fds_after}: the flap storm leaked fds"
        ));
    }
}

/// A1: 1,000 accept-then-drop flaps. Attempts stop at
/// [`MAX_RECONNECT_ATTEMPTS`], the first-to-last span covers the ladder
/// sum, no interval dips below the floor, and the handle/fd census is
/// clean.
fn case_thousand_flaps_capped() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(T0_MS / 1000);
    let mut engine = ReconnectEngine::new();
    let mut link = ScriptedLink::new(true, ScriptedRead::Dropped, true);
    let fds_before = fd_count();
    let mut failures = Vec::new();

    let flaps = drive_flaps(&mut engine, &mut link, &mut clock, &mut failures)?;
    if engine.parked() != Some(ParkReason::BackoffExhausted) {
        failures.push(format!(
            "parked = {:?}, want BackoffExhausted",
            engine.parked()
        ));
    }
    let times = engine.attempt_times_ms().to_vec();
    let floor = RECONNECT_LADDER_MS[0];
    for w in times.windows(2) {
        if w[1] - w[0] < floor {
            failures.push(format!(
                "inter-attempt interval {} ms below ladder floor {floor} ms",
                w[1] - w[0]
            ));
        }
    }
    let span_ms = times.last().copied().unwrap_or(0) - times.first().copied().unwrap_or(0);
    // 9 gaps: rungs 1..=9 = 100+500+2000+4000+8000+16000*4.
    let want_span = 100 + 500 + 2000 + 4000 + 8000 + 16000 * 4;
    if span_ms < want_span {
        failures.push(format!(
            "first-to-last span {span_ms} ms < ladder sum {want_span} ms"
        ));
    }
    check_census(&link, fds_before, &mut failures);
    let fds_after = fd_count();

    let evidence = vec![
        format!(
            "1000 flaps → {flaps} client attempts, parked: {:?}",
            engine.parked()
        ),
        format!(
            "min inter-attempt interval ≥ {floor} ms (ladder floor); span {span_ms} ms ≥ ladder sum {want_span} ms"
        ),
        format!(
            "link census: connects={} closes={} open={}; fds {fds_before} → {fds_after}",
            link.connect_count(),
            link.close_count(),
            link.open_handles()
        ),
    ];
    finish(
        CASES[0],
        serde_json::json!({
            "flaps": 1000,
            "attempts": flaps,
            "max_reconnect_attempts": MAX_RECONNECT_ATTEMPTS,
            "min_interval_ms": times.windows(2).map(|w| w[1] - w[0]).min().unwrap_or(0),
            "attempt_span_ms": span_ms,
            "fd_before": fds_before,
            "fd_after": fds_after,
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// Drive 7 boundary-synced flaps: accept, ack, abrupt drop exactly at
/// the healthy-stream boundary. Returns the next scheduled attempt.
/// One absolute-ms timeline: the first attempt goes at T0, every
/// deadline the engine returns is used verbatim.
fn drive_synced_flaps(
    engine: &mut ReconnectEngine,
    link: &mut ScriptedLink,
    clock: &mut ManualClock,
    failures: &mut Vec<String>,
) -> Result<u64, TaskDriverError> {
    let mut now_ms = T0_MS;
    for synced in 1..=7u32 {
        clock.set(now_ms / 1000);
        if !engine.note_attempt(now_ms) {
            failures.push(format!("synced flap {synced}: attempt refused"));
            break;
        }
        link.connect()
            .map_err(|e| arm_error("synced-flap", format!("accept failed: {e:?}")))?;
        // The stream is "acknowledged" here; the adversary drops it
        // exactly when the ladder would reset.
        let drop_at = now_ms + HEALTHY_STREAM_MS;
        clock.set(drop_at / 1000);
        link.close();
        let before = engine.consecutive_failures();
        engine.note_disconnect(drop_at, false); // abrupt: never a reset
        if engine.consecutive_failures() != before + 1 {
            failures.push(format!(
                "synced flap {synced}: counter {before} → {}, want {} (survive the reset point)",
                engine.consecutive_failures(),
                before + 1
            ));
        }
        now_ms = engine.next_attempt_at_ms().ok_or_else(|| {
            arm_error("synced-flap", format!("parked after synced flap {synced}"))
        })?;
    }
    if engine.consecutive_failures() != 7 {
        failures.push(format!(
            "counter = {}, want 7 after 7 synced flaps",
            engine.consecutive_failures()
        ));
    }
    engine.next_attempt_at_ms().ok_or_else(|| {
        arm_error(
            "synced-flap",
            "no attempt scheduled after 7 synced flaps".to_string(),
        )
    })
}

/// A genuine healthy + orderly close earns the reset the synced flaps
/// never got. Returns the post-reset delay.
fn earn_reset(
    engine: &mut ReconnectEngine,
    link: &mut ScriptedLink,
    clock: &mut ManualClock,
    scheduled: u64,
    floor: u64,
    failures: &mut Vec<String>,
) -> Result<u64, TaskDriverError> {
    clock.set(scheduled / 1000);
    if engine.note_attempt(scheduled) {
        link.connect()
            .map_err(|e| arm_error("healthy", format!("accept failed: {e:?}")))?;
        link.close();
        engine.note_disconnect(scheduled + HEALTHY_STREAM_MS + 1, true);
    }
    if engine.consecutive_failures() != 0 {
        failures.push(format!(
            "genuine healthy+orderly close did not reset: counter = {}",
            engine.consecutive_failures()
        ));
    }
    let delay_after_reset = engine
        .next_attempt_at_ms()
        .map(|at| at - (scheduled + HEALTHY_STREAM_MS + 1))
        .unwrap_or(u64::MAX);
    if delay_after_reset != floor {
        failures.push(format!(
            "post-reset delay {delay_after_reset} ms, want floor {floor} ms"
        ));
    }
    Ok(delay_after_reset)
}

/// A2: each flap is synced so the abrupt drop lands exactly on the
/// healthy-stream boundary — the moment a naive ladder would reset.
/// The consecutive-failure counter must survive: the ladder stays
/// escalated instead of collapsing to the floor interval. A genuine
/// healthy + orderly close then earns the reset, proving the counter
/// still resets when it should.
fn case_synced_flap_no_collapse() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(T0_MS / 1000);
    let mut engine = ReconnectEngine::new();
    let mut link = ScriptedLink::new(true, ScriptedRead::Dropped, true);
    let mut failures = Vec::new();
    let floor = RECONNECT_LADDER_MS[0];
    let top = RECONNECT_LADDER_MS[RECONNECT_LADDER_MS.len() - 1];

    let scheduled = drive_synced_flaps(&mut engine, &mut link, &mut clock, &mut failures)?;
    // 7 failures → ladder index min(7-1, 5) = top rung, not the floor:
    // the synced drops never collapsed the backoff. The last disconnect
    // was at last_attempt + HEALTHY_STREAM_MS, all absolute already.
    let last_attempt = engine.attempt_times_ms().last().copied().unwrap_or(0);
    let last_drop = last_attempt + HEALTHY_STREAM_MS;
    let delay = scheduled.saturating_sub(last_drop);
    if delay != top {
        failures.push(format!(
            "post-synced-flap delay {delay} ms, want top rung {top} ms (not floor {floor} ms)"
        ));
    }
    let delay_after_reset = earn_reset(
        &mut engine,
        &mut link,
        &mut clock,
        scheduled,
        floor,
        &mut failures,
    )?;

    let evidence = vec![
        format!(
            "7 boundary-synced abrupt drops → counter = {} (survived every reset point)",
            engine.consecutive_failures()
        ),
        format!(
            "ladder delay after synced flaps: {delay} ms (top rung {top}; floor would be {floor})"
        ),
        format!(
            "genuine healthy+orderly close → counter reset to 0, next delay {delay_after_reset} ms (floor)"
        ),
    ];
    finish(
        CASES[1],
        serde_json::json!({
            "synced_flaps": 7,
            "counter_after_synced": 7,
            "delay_after_synced_ms": delay,
            "delay_after_reset_ms": delay_after_reset,
            "ladder_floor_ms": floor,
            "ladder_top_ms": top,
            "reset_earned": delay_after_reset == floor,
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "thousand_flaps_capped" => case_thousand_flaps_capped(),
        "synced_flap_no_collapse" => case_synced_flap_no_collapse(),
        _ => Err(arm_error(
            "case",
            format!("task-162: unknown case '{case}'"),
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
            where_: "task-162".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-162".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
