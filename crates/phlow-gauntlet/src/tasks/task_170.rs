// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 170 — pairing code issuance (rust, V).
//!
//! The daemon mints Easy-Connect-style one-time codes:
//! `phlow-ec1:<base64url>` carrying a JSON payload with `issued_at`,
//! `ttl_secs == 900`, and the device `label`. The store keeps
//! SHA-256(secret) — never the secret — and a code presented within
//! its TTL with the correct secret pairs exactly once.
//!
//! The driver uses [`ManualClock`] (MOCK) for issue/present times and
//! the in-memory [`Daemon`] (no transport). The `phlow-ec1:` prefix,
//! the JSON shape, and the per-code store are ours; the 15-minute TTL
//! and hash-compared single-use secret adapt Ghostex
//! `server/src/remote_access/pairing_code.rs`.

use crate::bounty::clock::{Clock, ManualClock};
use crate::pairing::{
    CODE_PREFIX, Daemon, IssuedCode, PairingError, TTL_SECS, base64url_decode, sha256_hex,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-170";
/// Task name.
pub const NAME: &str = "pairing code issuance";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["issue_shape_and_hash_only", "present_pairs_once"];
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
        detail: format!("issue failed: {e:?}"),
    })
}

fn is_base64url(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// V1: the issued code has the exact `phlow-ec1:<base64url>` shape,
/// its payload JSON carries `issued_at`, `ttl_secs == 900`, and the
/// label that was passed in — and the store holds the secret's hash
/// and never the secret (the scan asserts the hash IS present as a
/// positive control, and the secret bytes are absent from the whole
/// serialized store).
/// Contract 1: the code has the `phlow-ec1:` prefix, a clean
/// base64url body (no padding), and the JSON payload carries the
/// issue timestamp, the fixed TTL, and the requested label.
fn check_code_shape(
    issued: &IssuedCode,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> (bool, bool, Option<u64>, Option<u64>, Option<String>) {
    let prefix_ok = issued.code.starts_with(CODE_PREFIX);
    if !prefix_ok {
        failures.push(format!(
            "code {:?} lacks the {CODE_PREFIX} prefix",
            issued.code
        ));
    }
    let body = issued.code.strip_prefix(CODE_PREFIX).unwrap_or("");
    let body_ok = is_base64url(body);
    if !body_ok {
        failures.push("code body is not clean base64url (no padding allowed)".to_string());
    }
    evidence.push(format!(
        "code shape: prefix_ok={prefix_ok} body_ok={body_ok} len={}",
        issued.code.len()
    ));

    let (issued_at, ttl_secs, label) = match base64url_decode(body) {
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(value) => (
                value.get("issued_at").and_then(|v| v.as_u64()),
                value.get("ttl_secs").and_then(|v| v.as_u64()),
                value
                    .get("label")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            ),
            Err(e) => {
                failures.push(format!("payload is not JSON: {e}"));
                (None, None, None)
            }
        },
        Err(e) => {
            failures.push(format!("payload body not base64url-decodable: {e:?}"));
            (None, None, None)
        }
    };
    if issued_at != Some(CLOCK_START) {
        failures.push(format!("issued_at={issued_at:?}, want {}", CLOCK_START));
    }
    if ttl_secs != Some(TTL_SECS) {
        failures.push(format!("ttl_secs={ttl_secs:?}, want {TTL_SECS}"));
    }
    if label.as_deref() != Some("pixel-9") {
        failures.push(format!("label={label:?}, want \"pixel-9\""));
    }
    evidence.push(format!(
        "payload: issued_at={issued_at:?} ttl_secs={ttl_secs:?} label={label:?}"
    ));
    if issued.expires_at != CLOCK_START + TTL_SECS {
        failures.push(format!(
            "expires_at={} want {}",
            issued.expires_at,
            CLOCK_START + TTL_SECS
        ));
    }
    (prefix_ok, body_ok, issued_at, ttl_secs, label)
}

/// Contract 2: the plaintext secret is absent from the whole
/// serialized store, while the secret's hash IS present —
/// the positive control, so the scan would find a leak.
fn check_hash_only(
    daemon: &Daemon,
    issued: &IssuedCode,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> (bool, bool, usize) {
    let dump = daemon.store_dump();
    let secret_present = dump
        .windows(issued.secret.len())
        .any(|window| window == issued.secret.as_slice());
    if secret_present {
        failures.push("store dump contains the plaintext secret".to_string());
    }
    let hash_hex = sha256_hex(&issued.secret);
    let dump_text = String::from_utf8_lossy(&dump);
    let hash_present = dump_text.contains(&hash_hex);
    if !hash_present {
        failures.push("store dump lacks the secret hash (positive control failed)".to_string());
    }
    evidence.push(format!(
        "store scan: dump_bytes={} secret_present={secret_present} \
         hash_present={hash_present}",
        dump.len()
    ));
    (secret_present, hash_present, dump.len())
}

fn case_issue_shape_and_hash_only() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let daemon = daemon()?;
    let issued = issue(&daemon, "pixel-9", clock.now())?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    let (prefix_ok, body_ok, issued_at, ttl_secs, label) =
        check_code_shape(&issued, &mut failures, &mut evidence);
    let (secret_present, hash_present, dump_bytes) =
        check_hash_only(&daemon, &issued, &mut failures, &mut evidence);

    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "prefix_ok": prefix_ok,
            "body_base64url": body_ok,
            "issued_at": issued_at,
            "ttl_secs": ttl_secs,
            "label": label,
            "expires_at": issued.expires_at,
            "secret_present": secret_present,
            "hash_present": hash_present,
            "dump_bytes": dump_bytes,
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// V2: presenting the code within the TTL with the correct secret
/// pairs the device exactly once — `Ok(device_id)`, one registered
/// device, and the code is consumed (a second present is refused).
fn case_present_pairs_once() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let daemon = daemon()?;
    let issued = issue(&daemon, "pixel-9", clock.now())?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    let mut secret = issued.secret.clone();
    let device_id = match daemon.verify(&issued.code, &mut secret, clock.now()) {
        Ok(id) => {
            evidence.push(format!("first present: PairingOk device_id={id}"));
            id
        }
        Err(e) => {
            failures.push(format!("first present failed: {e:?}, want PairingOk"));
            String::new()
        }
    };
    if daemon.device_count() != 1 {
        failures.push(format!("device_count={} want 1", daemon.device_count()));
    }
    match daemon.device_call(&device_id, "ping") {
        Ok(reply) if reply == "ok:ping" => {
            evidence.push("device_call after pairing: ok:ping".to_string());
        }
        other => failures.push(format!(
            "device_call after pairing: {other:?}, want ok:ping"
        )),
    }
    // The code is single-use: presenting it again is refused.
    let mut consumed_on_reuse = false;
    let mut secret2 = issued.secret.clone();
    match daemon.verify(&issued.code, &mut secret2, clock.now()) {
        Err(PairingError::Consumed) => {
            consumed_on_reuse = true;
            evidence.push("second present: Consumed (code marked consumed)".to_string());
        }
        other => failures.push(format!("second present: {other:?}, want Consumed")),
    }

    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "paired": !device_id.is_empty(),
            "device_count": daemon.device_count(),
            "consumed_on_reuse": consumed_on_reuse,
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
        "issue_shape_and_hash_only" => case_issue_shape_and_hash_only(),
        "present_pairs_once" => case_present_pairs_once(),
        _ => Err(TaskDriverError::Arm {
            arm: "case".to_string(),
            detail: format!("task-170: unknown case '{case}'"),
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
            where_: "task-170".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-170".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
