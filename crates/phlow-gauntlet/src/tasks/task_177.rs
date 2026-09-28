// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 177 — pairing ceremony integration + license audit (rust, V/A).
//!
//! V1 walks the full legitimate ceremony end to end on a fresh
//! daemon: issue → present within the TTL → one device registered →
//! the device's later daemon calls authenticate. V2 replays the
//! entire recorded transcript against a *fresh* daemon and requires
//! every step to fail and zero devices to register — the transcript
//! is the test's own honest enemy.
//!
//! The A case audits the attribution contract for Wave 28: every
//! Rust file carrying Ghostex-adapted code or doctrine must open
//! with the exact three-line header naming the Ghostex commit
//! `c91146607205ac49303d1bcfe2fd6f9a86741500` and stating the
//! re-implementation. This is the provenance receipt for tasks
//! 170–176.
//!
//! [`ManualClock`] (MOCK) drives time; the daemon is in-memory.

use crate::bounty::clock::{Clock, ManualClock};
use crate::pairing::{Daemon, PairingError};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-177";
/// Task name.
pub const NAME: &str = "pairing ceremony integration and license audit";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 1 validation, 1 adversarial.
pub const CASES: [&str; 2] = ["full_ceremony_registers_one", "license_headers_exact"];
/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 1_700_000_000;

/// The exact attribution header every Ghostex-adapted Rust file must
/// open with. Compared byte-for-byte, including the trailing newline.
pub const REQUIRED_HEADER: &str = "// Copyright (c) maddada\n// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500\n// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.\n";

/// Wave 28's Ghostex-adapted Rust files, relative to the
/// `phlow-gauntlet` crate root (the driver's working directory under
/// `cargo test`): the shared module, the eight drivers, and the eight
/// integration tests.
pub const ADAPTED_FILES: [&str; 17] = [
    "src/pairing.rs",
    "src/tasks/task_170.rs",
    "src/tasks/task_171.rs",
    "src/tasks/task_172.rs",
    "src/tasks/task_173.rs",
    "src/tasks/task_174.rs",
    "src/tasks/task_175.rs",
    "src/tasks/task_176.rs",
    "src/tasks/task_177.rs",
    "tests/task_170.rs",
    "tests/task_171.rs",
    "tests/task_172.rs",
    "tests/task_173.rs",
    "tests/task_174.rs",
    "tests/task_175.rs",
    "tests/task_176.rs",
    "tests/task_177.rs",
];

fn new_daemon() -> Result<Daemon, TaskDriverError> {
    Daemon::new().map_err(|e| TaskDriverError::Fixture {
        what: "daemon".to_string(),
        detail: format!("{e:?}"),
    })
}

/// One ceremony step, recorded for the replay case.
struct TranscriptStep {
    op: &'static str,
    code: String,
    secret: Vec<u8>,
    at: u64,
}

/// Steps 1–3, the live ceremony: the daemon issues a code for the
/// new phone; the phone presents code + secret at T+60; the
/// registered device carries its label and pairing time, and its
/// later calls authenticate while unknown device ids are refused.
/// Returns the device id and the recorded transcript.
fn run_ceremony(
    daemon: &Daemon,
    clock: &mut ManualClock,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(String, Vec<TranscriptStep>), TaskDriverError> {
    let issued = daemon
        .issue("pixel-9", clock.now())
        .map_err(|e| TaskDriverError::Arm {
            arm: "issue".to_string(),
            detail: format!("{e:?}"),
        })?;
    evidence.push(format!(
        "issued code for 'pixel-9', expires_at={}",
        issued.expires_at
    ));
    let transcript = vec![TranscriptStep {
        op: "present",
        code: issued.code.clone(),
        secret: issued.secret.clone(),
        at: clock.now() + 60,
    }];

    clock.advance(60);
    let mut secret = issued.secret.clone();
    let device_id = match daemon.verify(&issued.code, &mut secret, clock.now()) {
        Ok(id) => {
            evidence.push(format!("present at T+60s: PairingOk device_id={id}"));
            id
        }
        Err(e) => {
            failures.push(format!("present at T+60s: {e:?}, want PairingOk"));
            String::new()
        }
    };
    if daemon.device_count() != 1 {
        failures.push(format!(
            "device_count={} want exactly 1",
            daemon.device_count()
        ));
    }
    match daemon.device_info(&device_id) {
        Some((label, paired_at)) if label == "pixel-9" && paired_at == CLOCK_START + 60 => {
            evidence.push("device_info: label='pixel-9' paired_at=T+60s".to_string());
        }
        other => failures.push(format!("device_info={other:?}, want ('pixel-9', T+60s)")),
    }
    match daemon.device_call(&device_id, "list-commands") {
        Ok(reply) if reply == "ok:list-commands" => {
            evidence.push("paired device call: ok:list-commands".to_string());
        }
        other => failures.push(format!("paired device call: {other:?}, want ok")),
    }
    match daemon.device_call("not-a-device", "list-commands") {
        Err(_) => evidence.push("unknown device call: refused".to_string()),
        Ok(reply) => failures.push(format!(
            "unknown device call answered {reply:?}, want refusal"
        )),
    }
    Ok((device_id, transcript))
}

/// Step 4: the transcript replayed against a FRESH daemon must fail
/// at every step. A fresh daemon holds a different instance key, so
/// the replayed codes die at the MAC gate as `Authenticity` — the
/// transcript is bound to the daemon that minted it, and registers
/// zero devices. Returns (refused steps, devices on the fresh
/// daemon) — the count stays honest even on the failure path.
fn replay_transcript_foreign(
    transcript: &[TranscriptStep],
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(usize, usize), TaskDriverError> {
    let fresh = new_daemon()?;
    let mut replay_failures = 0usize;
    for step in transcript {
        let mut secret = step.secret.clone();
        match fresh.verify(&step.code, &mut secret, step.at) {
            Err(PairingError::Authenticity) => {
                replay_failures += 1;
                evidence.push(format!(
                    "replay on fresh daemon: {} refused as Authenticity (foreign instance key)",
                    step.op
                ));
            }
            other => failures.push(format!(
                "replay on fresh daemon: {} -> {other:?}, want Authenticity",
                step.op
            )),
        }
    }
    if replay_failures != transcript.len() {
        failures.push(format!(
            "transcript replay: {replay_failures}/{} steps refused",
            transcript.len()
        ));
    }
    if fresh.device_count() != 0 {
        failures.push(format!(
            "transcript replay registered {} devices on the fresh daemon, want 0",
            fresh.device_count()
        ));
    } else {
        evidence.push("transcript replay: 0 devices registered on fresh daemon".to_string());
    }
    Ok((replay_failures, fresh.device_count()))
}

/// V1: the full legitimate ceremony. Issue at T+0, present at T+60
/// with the right secret: one device registers, and that device's
/// later calls authenticate while an unknown id is refused. The
/// transcript of the ceremony is recorded for the replay case.
fn case_full_ceremony_registers_one() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(CLOCK_START);
    let daemon = new_daemon()?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    let (_device_id, transcript) = run_ceremony(&daemon, &mut clock, &mut failures, &mut evidence)?;
    let (replay_failures, replay_devices) =
        replay_transcript_foreign(&transcript, &mut failures, &mut evidence)?;

    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "devices_registered": daemon.device_count(),
            "paired_call_ok": true,
            "unknown_call_refused": true,
            "replay_steps_refused": replay_failures,
            "replay_devices": replay_devices,
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// A: every Wave 28 adapted file opens with the exact attribution
/// header, byte-for-byte — the Ghostex commit named, the
/// re-implementation stated. A missing or altered header fails the
/// case and names the file.
fn case_license_headers_exact() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut checked = 0usize;

    for file in ADAPTED_FILES {
        let text = std::fs::read_to_string(file).map_err(|e| TaskDriverError::Fixture {
            what: format!("read {file}"),
            detail: e.to_string(),
        })?;
        checked += 1;
        if text.starts_with(REQUIRED_HEADER) {
            evidence.push(format!("{file}: header exact"));
        } else {
            let head: String = text.chars().take(200).collect();
            failures.push(format!("{file}: header mismatch; file opens with {head:?}"));
        }
    }

    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "files_checked": checked,
            "files_exact": checked - failures.len(),
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
        "full_ceremony_registers_one" => case_full_ceremony_registers_one(),
        "license_headers_exact" => case_license_headers_exact(),
        _ => Err(TaskDriverError::Arm {
            arm: "case".to_string(),
            detail: format!("task-177: unknown case '{case}'"),
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
            where_: "task-177".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-177".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
