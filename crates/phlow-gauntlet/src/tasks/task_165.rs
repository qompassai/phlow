// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 165 — worker shutdown without zombies (rust, V/A).
//!
//! Shutdown is total: the worker thread is joined, in-flight messages
//! are drained-or-dropped per the declared policy, no thread/fd
//! survives the client. V1: clean shutdown of an idle worker → the
//! thread joins within 1 s, the fd count is back to baseline. A1:
//! shutdown with 50 messages in flight against an unresponsive daemon →
//! shutdown still completes within [`SHUTDOWN_TIMEOUT`](crate::daemon_client::SHUTDOWN_TIMEOUT),
//! every message's disposition (drained vs dropped) is logged, and the
//! thread census shows zero worker threads afterwards. The worker is
//! the real threaded
//! [`DaemonClient`](crate::daemon_client::DaemonClient) over a
//! [`ScriptedLink`](crate::daemon_client::ScriptedLink) double (MOCK);
//! the census is the real thread list and fd count.
//!
//! Time note: join bounds are wall-clock by necessity — a parked thread
//! cannot observe a scripted clock. Tasks 159–164 carry the
//! [`ManualClock`](crate::bounty::clock::ManualClock) determinism; this
//! task measures real shutdown latency against real bounds.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::daemon_client::{
    DaemonClient, SHUTDOWN_TIMEOUT, ScriptedLink, ScriptedRead, threads_named,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-165";
/// Task name.
pub const NAME: &str = "worker shutdown without zombies";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 1 validation + 1 adversarial.
pub const CASES: [&str; 2] = ["idle_shutdown_joins", "unresponsive_daemon_fifty_inflight"];
/// Idle-shutdown join bar.
const IDLE_JOIN_BAR: Duration = Duration::from_secs(1);
/// Time for the worker to connect and settle before shutdown.
const SETTLE: Duration = Duration::from_millis(300);

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

fn lock_probe(probe: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    probe.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
    let mut report_out = CaseReport::pass(case, metrics, full_evidence);
    report_out.failures = failures;
    report_out.passed = report_out.failures.is_empty();
    Ok(report_out)
}

/// V1: idle worker → shutdown joins within 1 s, fds back to baseline,
/// no worker thread left. The probe proves the worker actually ran its
/// connect/subscribe loop before shutdown (not a stillborn thread).
fn case_idle_shutdown_joins() -> Result<CaseReport, TaskDriverError> {
    let probe = Arc::new(Mutex::new(Vec::new()));
    let mut link = ScriptedLink::with_probe(true, ScriptedRead::Timeout, true, Arc::clone(&probe));
    link.script_reads(vec![ScriptedRead::Frame("ack".to_string())]);
    let mut failures = Vec::new();

    let client = DaemonClient::spawn(Box::new(link), &["a"])
        .map_err(|e| arm_error("spawn", format!("worker spawn failed: {e}")))?;
    std::thread::sleep(SETTLE);
    let report = client.shutdown();
    if !report.joined {
        failures.push("worker thread was not joined".to_string());
    }
    if report.elapsed > IDLE_JOIN_BAR {
        failures.push(format!(
            "idle shutdown took {:?}, bar is {IDLE_JOIN_BAR:?}",
            report.elapsed
        ));
    }
    if !report.dispositions.is_empty() {
        failures.push(format!(
            "idle shutdown logged dispositions: {:?}",
            report.dispositions
        ));
    }
    if report.fd_delta != 0 {
        failures.push(format!("fd delta {}, want 0", report.fd_delta));
    }
    if threads_named("phlow-daemon-") != 0 {
        failures.push("worker thread survived shutdown (thread census)".to_string());
    }
    let written = lock_probe(&probe);
    if !written.iter().any(|f| f == "sub:a") {
        failures.push(format!(
            "worker never ran its subscribe loop; probe saw {written:?}"
        ));
    }

    let evidence = vec![
        format!(
            "idle shutdown: joined = {}, elapsed = {:?}",
            report.joined, report.elapsed
        ),
        format!(
            "dispositions: {:?} (want none); fd delta: {}",
            report.dispositions, report.fd_delta
        ),
        format!(
            "worker threads after shutdown: {}",
            threads_named("phlow-daemon-")
        ),
        format!(
            "probe saw subscribe frame: {}",
            written.iter().any(|f| f == "sub:a")
        ),
    ];
    finish(
        CASES[0],
        serde_json::json!({
            "joined": report.joined,
            "elapsed_ms": report.elapsed.as_millis(),
            "idle_join_bar_ms": IDLE_JOIN_BAR.as_millis(),
            "fd_delta": report.fd_delta,
            "worker_threads_after": threads_named("phlow-daemon-"),
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// Queue 50 messages on the live worker, refusing to accept silent
/// send failures.
fn send_fifty(client: &DaemonClient, failures: &mut Vec<String>) {
    for i in 0..50u32 {
        if !client.send(format!("msg-{i}")) {
            failures.push(format!("send msg-{i} refused while worker alive"));
        }
    }
}

/// Every in-flight message gets exactly one disposition, in order,
/// each a drop that names its reason.
fn check_dispositions(dispositions: &[String], failures: &mut Vec<String>) {
    if dispositions.len() != 50 {
        failures.push(format!(
            "{} dispositions logged, want exactly 50 (one per in-flight message)",
            dispositions.len()
        ));
    }
    for (i, d) in dispositions.iter().enumerate() {
        if !d.starts_with(&format!("dropped msg-{i}")) {
            failures.push(format!("disposition {i} out of order or not a drop: {d}"));
            break;
        }
        if !d.contains("WriteTimeout") {
            failures.push(format!("disposition {i} names no reason: {d}"));
            break;
        }
    }
}

/// A1: 50 messages in flight, daemon unresponsive (every bounded write
/// times out). Shutdown completes within [`SHUTDOWN_TIMEOUT`]; each
/// message is logged as dropped with its reason; the thread is joined
/// and no worker thread survives.
fn case_unresponsive_daemon_fifty_inflight() -> Result<CaseReport, TaskDriverError> {
    let probe = Arc::new(Mutex::new(Vec::new()));
    let link = ScriptedLink::with_probe(true, ScriptedRead::Timeout, false, Arc::clone(&probe));
    let mut failures = Vec::new();

    let client = DaemonClient::spawn(Box::new(link), &[])
        .map_err(|e| arm_error("spawn", format!("worker spawn failed: {e}")))?;
    std::thread::sleep(SETTLE);
    send_fifty(&client, &mut failures);
    let report = client.shutdown();
    if !report.joined {
        failures.push("worker thread was not joined".to_string());
    }
    if report.elapsed > SHUTDOWN_TIMEOUT {
        failures.push(format!(
            "shutdown took {:?}, bound is {SHUTDOWN_TIMEOUT:?}",
            report.elapsed
        ));
    }
    check_dispositions(&report.dispositions, &mut failures);
    if threads_named("phlow-daemon-") != 0 {
        failures.push("worker thread survived shutdown (thread census)".to_string());
    }
    if report.fd_delta != 0 {
        failures.push(format!("fd delta {}, want 0", report.fd_delta));
    }

    let evidence = vec![
        format!(
            "50 in-flight vs unresponsive daemon: shutdown elapsed = {:?} (bound {SHUTDOWN_TIMEOUT:?})",
            report.elapsed
        ),
        format!(
            "dispositions: {} (all dropped, in order, reason WriteTimeout)",
            report.dispositions.len()
        ),
        format!(
            "first: {:?}; last: {:?}",
            report.dispositions.first(),
            report.dispositions.last()
        ),
        format!(
            "worker threads after shutdown: {}",
            threads_named("phlow-daemon-")
        ),
    ];
    finish(
        CASES[1],
        serde_json::json!({
            "in_flight": 50,
            "dispositions": report.dispositions.len(),
            "all_dropped_in_order": report.dispositions.iter().enumerate().all(|(i, d)| d.starts_with(&format!("dropped msg-{i}"))),
            "elapsed_ms": report.elapsed.as_millis(),
            "shutdown_timeout_ms": SHUTDOWN_TIMEOUT.as_millis(),
            "joined": report.joined,
            "worker_threads_after": threads_named("phlow-daemon-"),
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "idle_shutdown_joins" => case_idle_shutdown_joins(),
        "unresponsive_daemon_fifty_inflight" => case_unresponsive_daemon_fifty_inflight(),
        _ => Err(arm_error(
            "case",
            format!("task-165: unknown case '{case}'"),
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
            where_: "task-165".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-165".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
