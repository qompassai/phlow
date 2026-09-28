// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 164 — half-open connection timeout (rust, A).
//!
//! A peer that connects and then stalls (slow-loris) must not hold a
//! worker slot forever. A1: one peer sends 3 bytes of the 4-byte frame
//! header and stalls → [`FRAME_READ_TIMEOUT`](crate::daemon_client::FRAME_READ_TIMEOUT)
//! fires, the slot is released, the fd is closed, and no handler thread
//! is left parked. A2: 100 such peers at once → all reaped within the
//! timeout bound, and a legitimate client is still accepted afterwards.
//! The [`DaemonFixture`](crate::daemon_client::DaemonFixture) (MOCK) is
//! a real loopback-TCP daemon; stalled peers are real TCP sockets held
//! open by the test; the census is the real fd count and thread list.

use std::io::Write;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::daemon_client::{
    DaemonFixture, FRAME_READ_TIMEOUT, fd_count, send_frame, threads_named,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-164";
/// Task name.
pub const NAME: &str = "half-open connection timeout";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = ["three_byte_stall_reaped", "hundred_stallers_reaped"];
/// Fixture credential for the legitimate client.
const TOKEN: &str = "fixture-token-164";
/// How long the tests wait for the reaper beyond the read deadline.
const REAP_SLACK: Duration = Duration::from_millis(600);
/// Bound for reaping all stalled peers in A2.
const REAP_BOUND: Duration = Duration::from_secs(4);

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// A stalled peer: connects, writes 3 bytes of the frame header, then
/// holds the socket open without sending anything more.
fn stalled_peer(fixture: &DaemonFixture) -> Result<TcpStream, TaskDriverError> {
    let mut peer = TcpStream::connect(fixture.addr())
        .map_err(|e| arm_error("peer", format!("connect failed: {e}")))?;
    peer.write_all(b"abc")
        .map_err(|e| arm_error("peer", format!("3-byte write failed: {e}")))?;
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| arm_error("peer", format!("set_read_timeout failed: {e}")))?;
    Ok(peer)
}

/// A1: one 3-byte stall → read deadline fires, slot released, fd
/// closed, no parked handler thread.
fn case_three_byte_stall_reaped() -> Result<CaseReport, TaskDriverError> {
    let fixture = DaemonFixture::start(TOKEN, 1, 0)
        .map_err(|e| arm_error("daemon", format!("fixture failed to start: {e}")))?;
    let mut failures = Vec::new();

    let fds_before = fd_count();
    let threads_before = threads_named("phlow-daemon-");
    let _holder = stalled_peer(&fixture)?;
    // The daemon must reap the stall on its own read deadline; the test
    // only waits long enough for the deadline plus slack to pass, then
    // releases its side before the census.
    std::thread::sleep(FRAME_READ_TIMEOUT + REAP_SLACK);
    drop(_holder);
    std::thread::sleep(Duration::from_millis(100));
    let fds_after = fd_count();
    let threads_after = threads_named("phlow-daemon-");
    let slots = fixture.slots();
    if slots != 0 {
        failures.push(format!(
            "{slots} accept-path slots still held after the read deadline"
        ));
    }
    if fds_after != fds_before {
        failures.push(format!(
            "fd count {fds_before} → {fds_after}: the stalled peer's fd was not closed"
        ));
    }
    if threads_after != threads_before {
        failures.push(format!(
            "daemon threads {threads_before} → {threads_after}: a handler thread is parked"
        ));
    }
    let audit = fixture.audit();
    if !audit.iter().any(|l| l.contains("read timeout")) {
        failures.push(format!("no read-timeout audit line; audit: {audit:?}"));
    }
    fixture.stop();

    let evidence = vec![
        format!("3-byte stall → reaped after {FRAME_READ_TIMEOUT:?} + slack"),
        format!(
            "slots: {slots} (want 0); fds {fds_before} → {fds_after}; daemon threads {threads_before} → {threads_after}"
        ),
        format!(
            "audit has read-timeout line: {}",
            audit.iter().any(|l| l.contains("read timeout"))
        ),
    ];
    finish(
        CASES[0],
        serde_json::json!({
            "slots_held": slots,
            "fd_before": fds_before,
            "fd_after": fds_after,
            "threads_before": threads_before,
            "threads_after": threads_after,
            "read_timeout_logged": true,
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// A2: 100 stalled peers at once → all reaped within the bound; the fd
/// count returns to baseline; a legitimate client is accepted after.
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

/// Connect `n` stalled peers, returning the sockets the test holds.
fn stall_peers(fixture: &DaemonFixture, n: usize, failures: &mut Vec<String>) -> Vec<TcpStream> {
    let mut holders = Vec::with_capacity(n);
    for i in 0..n {
        match stalled_peer(fixture) {
            Ok(peer) => holders.push(peer),
            Err(e) => {
                failures.push(format!("staller {i} failed to connect: {e}"));
                break;
            }
        }
    }
    holders
}

/// Wait for every accept-path slot to release, bounded. Returns the
/// elapsed wait and the remaining slot count.
fn wait_for_reap(fixture: &DaemonFixture, failures: &mut Vec<String>) -> (Duration, usize) {
    let reap_start = Instant::now();
    while fixture.slots() > 0 && reap_start.elapsed() < REAP_BOUND {
        std::thread::sleep(Duration::from_millis(20));
    }
    let reap_elapsed = reap_start.elapsed();
    let slots = fixture.slots();
    if slots != 0 {
        failures.push(format!("{slots} slots still held after {reap_elapsed:?}"));
    }
    if reap_elapsed > FRAME_READ_TIMEOUT + Duration::from_secs(3) {
        failures.push(format!(
            "reap took {reap_elapsed:?}, bound is {FRAME_READ_TIMEOUT:?} + 3 s"
        ));
    }
    (reap_elapsed, slots)
}

/// A legitimate client must still be accepted after the storm.
fn check_legitimate(
    fixture: &DaemonFixture,
    failures: &mut Vec<String>,
) -> Result<serde_json::Value, TaskDriverError> {
    let reply = send_frame(
        fixture.addr(),
        br#"{"op":"subscribe","topic":"legit","cred":"fixture-token-164"}"#,
    )
    .map_err(|e| arm_error("legit", format!("legitimate client failed: {e}")))?;
    let reply_json: serde_json::Value = serde_json::from_slice(&reply)
        .map_err(|e| arm_error("legit", format!("reply is not JSON: {e}")))?;
    if reply_json.get("ok").and_then(|o| o.as_str()) != Some("subscribed") {
        failures.push(format!("legitimate subscribe gave {reply_json}"));
    }
    if !fixture.subscribes().contains(&"legit".to_string()) {
        failures.push(format!(
            "daemon did not record the legitimate subscribe: {:?}",
            fixture.subscribes()
        ));
    }
    Ok(reply_json)
}

fn case_hundred_stallers_reaped() -> Result<CaseReport, TaskDriverError> {
    let fixture = DaemonFixture::start(TOKEN, 1, 0)
        .map_err(|e| arm_error("daemon", format!("fixture failed to start: {e}")))?;
    let mut failures = Vec::new();

    let fds_before = fd_count();
    let holders = stall_peers(&fixture, 100, &mut failures);
    let (reap_elapsed, slots) = wait_for_reap(&fixture, &mut failures);
    // The test held its side of all 100 sockets; dropping them must
    // restore the exact baseline fd count.
    drop(holders);
    std::thread::sleep(Duration::from_millis(100));
    let fds_after = fd_count();
    if fds_after != fds_before {
        failures.push(format!(
            "fd count {fds_before} → {fds_after} after reaping 100 stallers"
        ));
    }
    let reply_json = check_legitimate(&fixture, &mut failures)?;
    fixture.stop();

    let evidence = vec![
        format!(
            "100 stalled peers → all reaped in {reap_elapsed:?} (bound {FRAME_READ_TIMEOUT:?} + 3 s)"
        ),
        format!("slots: {slots} (want 0); fds {fds_before} → {fds_after}"),
        format!("legitimate client after the storm: {reply_json}"),
    ];
    finish(
        CASES[1],
        serde_json::json!({
            "stallers": 100,
            "slots_held": slots,
            "reap_elapsed_ms": reap_elapsed.as_millis(),
            "fd_before": fds_before,
            "fd_after": fds_after,
            "legitimate_accepted": reply_json.get("ok").and_then(|o| o.as_str()) == Some("subscribed"),
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "three_byte_stall_reaped" => case_three_byte_stall_reaped(),
        "hundred_stallers_reaped" => case_hundred_stallers_reaped(),
        _ => Err(arm_error(
            "case",
            format!("task-164: unknown case '{case}'"),
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
            where_: "task-164".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-164".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
