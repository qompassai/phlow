// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 163 — hostile peer on the daemon socket (rust, A).
//!
//! Any local process can connect to the daemon socket, so the daemon
//! must not trust the peer: loopback is not trust. A1 has a hostile
//! peer send well-formed privileged frames with no (or a wrong)
//! credential → typed `AuthError`, connection dropped, attempt logged,
//! zero privileged effects. A2 replays a captured *valid* snapshot
//! frame — right credential, but a version/`as_of` behind the daemon's
//! current epoch → typed `StaleSnapshot` rejection, daemon state
//! unchanged. The [`DaemonFixture`](crate::daemon_client::DaemonFixture)
//! (MOCK) is a real loopback-TCP daemon implementing the accept path
//! under test; the hostile peer is a real TCP client.

use crate::daemon_client::{DaemonFixture, send_frame};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-163";
/// Task name.
pub const NAME: &str = "hostile peer on the daemon socket";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = [
    "unauthenticated_privileged_refused",
    "replayed_snapshot_stale",
];
/// Fixture credential; the hostile peer never learns it.
const TOKEN: &str = "fixture-token-163";

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// Send one frame, parse the JSON reply.
fn probe(fixture: &DaemonFixture, payload: &[u8]) -> Result<serde_json::Value, TaskDriverError> {
    let raw = send_frame(fixture.addr(), payload)
        .map_err(|e| arm_error("peer", format!("frame round-trip failed: {e}")))?;
    serde_json::from_slice(&raw).map_err(|e| arm_error("peer", format!("reply is not JSON: {e}")))
}

fn err_of(reply: &serde_json::Value) -> Option<&str> {
    reply.get("err").and_then(|e| e.as_str())
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

/// One replay probe: send a frame, assert the typed error in the reply.
fn expect_err(
    fixture: &DaemonFixture,
    payload: &[u8],
    want: &str,
    failures: &mut Vec<String>,
) -> Result<serde_json::Value, TaskDriverError> {
    let reply = probe(fixture, payload)?;
    if err_of(&reply) != Some(want) {
        failures.push(format!("replay gave {reply}, want {want}"));
    }
    Ok(reply)
}

/// A1: well-formed privileged frames without a credential are refused
/// with `AuthError`, dropped, and logged; zero privileged effects run.
/// A correctly credentialed `ping` proves the refusal is about
/// authentication, not framing.
fn case_unauthenticated_privileged_refused() -> Result<CaseReport, TaskDriverError> {
    let fixture = DaemonFixture::start(TOKEN, 5, 100)
        .map_err(|e| arm_error("daemon", format!("fixture failed to start: {e}")))?;
    fixture.seed_state(&[("k", "v5")]);
    let mut failures = Vec::new();

    // Privileged frame, no credential at all.
    let r1 = probe(&fixture, br#"{"op":"daemon-shutdown"}"#)?;
    if err_of(&r1) != Some("AuthError") {
        failures.push(format!(
            "cred-less daemon-shutdown gave {r1}, want AuthError"
        ));
    }
    // Privileged frame, wrong credential.
    let r2 = probe(&fixture, br#"{"op":"exec","argv":["id"],"cred":"wrong"}"#)?;
    if err_of(&r2) != Some("AuthError") {
        failures.push(format!("wrong-cred exec gave {r2}, want AuthError"));
    }
    // Zero privileged effects from either unauthenticated peer.
    let effects = fixture.effects();
    if !effects.is_empty() {
        failures.push(format!("privileged effects executed: {effects:?}"));
    }
    // Positive control: the same framing with the right credential works.
    let r3 = probe(&fixture, br#"{"op":"ping","cred":"fixture-token-163"}"#)?;
    if r3.get("ok").and_then(|o| o.as_str()) != Some("pong") {
        failures.push(format!("credentialed ping gave {r3}, want ok/pong"));
    }
    // The attempts were logged.
    let audit = fixture.audit();
    let auth_logs = audit.iter().filter(|l| l.contains("AuthError")).count();
    if auth_logs < 2 {
        failures.push(format!(
            "{auth_logs} AuthError audit lines, want >= 2; audit: {audit:?}"
        ));
    }
    fixture.stop();

    let evidence = vec![
        format!("cred-less daemon-shutdown → {r1}"),
        format!("wrong-cred exec → {r2}"),
        format!("privileged effects executed: {effects:?} (want none)"),
        format!("credentialed ping → {r3} (framing is fine; auth is the gate)"),
        format!("AuthError audit lines: {auth_logs}"),
    ];
    finish(
        CASES[0],
        serde_json::json!({
            "auth_errors": auth_logs,
            "privileged_effects": effects.len(),
            "positive_control_ok": r3.get("ok").and_then(|o| o.as_str()) == Some("pong"),
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// A2: a captured *valid* snapshot frame (right credential, but
/// version 4 / as_of 90 behind the daemon's 5/100) is replayed →
/// typed `StaleSnapshot`, daemon state unchanged. A credential-less
/// replay is still `AuthError`: authentication is checked first.
fn case_replayed_snapshot_stale() -> Result<CaseReport, TaskDriverError> {
    let fixture = DaemonFixture::start(TOKEN, 5, 100)
        .map_err(|e| arm_error("daemon", format!("fixture failed to start: {e}")))?;
    fixture.seed_state(&[("k", "v5")]);
    let mut failures = Vec::new();
    let state_before = fixture.state_snapshot();

    // Replay: valid credential, stale epoch.
    let replay = br#"{"op":"snapshot","version":4,"as_of":90,"state":{"k":"old"},"cred":"fixture-token-163"}"#;
    let r1 = expect_err(&fixture, replay, "StaleSnapshot", &mut failures)?;
    if r1.get("current_version").and_then(|v| v.as_u64()) != Some(5) {
        failures.push(format!(
            "StaleSnapshot did not name current_version=5: {r1}"
        ));
    }
    // Same stale frame, same epoch, but as_of-only staleness.
    let r2 = expect_err(
        &fixture,
        br#"{"op":"snapshot","version":5,"as_of":95,"state":{"k":"old"},"cred":"fixture-token-163"}"#,
        "StaleSnapshot",
        &mut failures,
    )?;
    // Credential-less replay: AuthError, not StaleSnapshot — auth first.
    let r3 = expect_err(
        &fixture,
        br#"{"op":"snapshot","version":4,"as_of":90,"state":{"k":"old"}}"#,
        "AuthError",
        &mut failures,
    )?;
    let audit = fixture.audit();
    let stale_logs = audit.iter().filter(|l| l.contains("StaleSnapshot")).count();
    if stale_logs < 2 {
        failures.push(format!(
            "{stale_logs} StaleSnapshot audit lines, want >= 2; audit: {audit:?}"
        ));
    }
    let state_unchanged = fixture.state_snapshot() == state_before;
    if !state_unchanged {
        failures.push("daemon state changed during replay probes".to_string());
    }
    fixture.stop();

    let evidence = vec![
        format!("replayed valid snapshot (v4/as_of 90, good cred) → {r1}"),
        format!("as_of-stale replay (v5/as_of 95) → {r2}"),
        format!("cred-less replay → {r3} (auth before staleness)"),
        format!("daemon state before == after: {state_unchanged}"),
    ];
    finish(
        CASES[1],
        serde_json::json!({
            "stale_rejections": stale_logs,
            "state_unchanged": state_unchanged,
            "auth_before_staleness": err_of(&r3) == Some("AuthError"),
            "backend": "scripted-mock",
        }),
        evidence,
        failures,
    )
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "unauthenticated_privileged_refused" => case_unauthenticated_privileged_refused(),
        "replayed_snapshot_stale" => case_replayed_snapshot_stale(),
        _ => Err(arm_error(
            "case",
            format!("task-163: unknown case '{case}'"),
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
            where_: "task-163".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-163".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
