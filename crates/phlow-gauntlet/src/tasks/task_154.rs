// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 154 — versioned envelope routing (rust, V).
//!
//! The seam is `envelope.version` → parser dispatch. Every envelope
//! carries a version; the receiver routes to the matching parser and
//! rejects what it cannot handle with a typed error — never a panic,
//! never a silent misparse. Adapted from the Ghostex versioned
//! envelope rule (`rpc.rs`'s `protocolVersion` gate, `event.rs`'s
//! `EventHeader`); unlike Ghostex's exact-match gate, this build
//! routes a *range* of versions and types every refusal. The driver
//! uses a scripted version-skewed peer (MOCK) against
//! [`crate::wire::route_version`].

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::wire::{EditorSocket, VersionError, parse_envelope, route_version};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-154";
/// Task name.
pub const NAME: &str = "versioned envelope routing";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 3 validation (the 3rd is the license gate).
pub const CASES: [&str; 3] = [
    "interleaved_versions_routed",
    "too_old_and_missing_typed",
    "license_header_present",
];

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// One scripted envelope at `version` from the peer (MOCK).
fn envelope_json(version: u64) -> String {
    format!(r#"{{"version":{version},"kind":"event","id":"s1","body":{{"v":{version}}}}}"#)
}

/// V1: v1 and v2 envelopes interleaved on one stream are each routed
/// to their version, and each response carries the matching version.
/// The connection stays up throughout.
fn case_interleaved_versions_routed() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let socket = EditorSocket::new();
    let versions = [1u64, 2, 1, 2, 1, 2];
    let mut routed = Vec::new();
    let mut responses = Vec::new();
    for v in versions {
        let bytes = envelope_json(v);
        let (env, _) = parse_envelope(bytes.as_bytes())
            .map_err(|e| arm_error("parse", format!("task-154: v{v} envelope refused: {e:?}")))?;
        let routed_v = route_version(env.version)
            .map_err(|e| arm_error("route", format!("task-154: v{v} failed routing: {e:?}")))?;
        routed.push(routed_v);
        // The response carries the matching version back.
        responses.push(format!(
            r#"{{"version":{routed_v},"kind":"response","id":"s1"}}"#
        ));
        if !socket.is_alive() {
            failures.push(format!("connection dropped while routing v{v}"));
        }
    }
    if routed != versions {
        failures.push(format!("routed {routed:?}, want {versions:?}"));
    }
    for (i, resp) in responses.iter().enumerate() {
        let (env, _) = parse_envelope(resp.as_bytes())
            .map_err(|e| arm_error("parse", format!("task-154: response {i} refused: {e:?}")))?;
        if env.version != Some(versions[i]) {
            failures.push(format!(
                "response {i} carries version {:?}, want {}",
                env.version, versions[i]
            ));
        }
    }
    if !socket.is_alive() {
        failures.push("connection dropped after interleaved stream".to_string());
    }
    let evidence = vec![format!(
        "interleaved stream routed as {routed:?}; 6 responses carry matching versions; connection up"
    )];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "routed": routed,
            "responses": responses.len(),
            "connection_up": socket.is_alive(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: version 0 (below minimum) → `VersionError::TooOld`; a missing
/// version field → `VersionError::Missing`. The connection stays up
/// after both refusals.
fn case_too_old_and_missing_typed() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut socket = EditorSocket::new();
    // Version 0: parses fine as an envelope, fails at routing.
    let v0 = r#"{"version":0,"kind":"ping","id":"s1"}"#;
    match socket.read(v0.as_bytes()) {
        Err(crate::wire::SocketError::Version(VersionError::TooOld { got: 0, min: 1 })) => {}
        other => failures.push(format!("version 0 gave {other:?}, want TooOld")),
    }
    // Missing version: same path, different typed error.
    let no_v = r#"{"kind":"ping","id":"s1"}"#;
    match socket.read(no_v.as_bytes()) {
        Err(crate::wire::SocketError::Version(VersionError::Missing)) => {}
        other => failures.push(format!("missing version gave {other:?}, want Missing")),
    }
    if !socket.is_alive() {
        failures.push("connection dropped on typed version refusal".to_string());
    }
    if socket.rejected() != 2 {
        failures.push(format!("rejected {}, want 2", socket.rejected()));
    }
    // The connection is still usable: a good envelope reads cleanly.
    match socket.read(envelope_json(1).as_bytes()) {
        Ok(routed) if routed.version == 1 => {}
        other => failures.push(format!("good envelope after refusals gave {other:?}")),
    }
    let evidence = vec![
        "version 0 -> VersionError::TooOld; missing version -> VersionError::Missing".to_string(),
        format!(
            "connection up after both refusals (rejected={}); good envelope still reads",
            socket.rejected()
        ),
    ];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "too_old_typed": true,
            "missing_typed": true,
            "rejected": socket.rejected(),
            "received": socket.received(),
            "connection_up": socket.is_alive(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// License gate: the adapted wire module carries the maddada
/// attribution and the source commit.
fn case_license_header_present() -> Result<CaseReport, TaskDriverError> {
    crate::tasks::task_158::check_attribution(&["src/wire.rs", "src/tasks/task_154.rs"])
        .map(|mut r| {
            r.case = CASES[2].to_string();
            r
        })
        .map_err(|e| arm_error("license", e))
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "interleaved_versions_routed" => case_interleaved_versions_routed(),
        "too_old_and_missing_typed" => case_too_old_and_missing_typed(),
        "license_header_present" => case_license_header_present(),
        _ => Err(arm_error(
            "case",
            format!("task-154: unknown case '{case}'"),
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
            where_: "task-154".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-154".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
