//! Task 146 — operator-approval boundary on submission (rust, A).
//!
//! The seam is [`SubmissionGate::submit`]: the last human checkpoint.
//! No auto-submit, ever — even an all-green finding submits only with
//! a live operator approval bound to the exact payload bytes, and each
//! approval nonce is single-use (replay refused). Four scenarios: a
//! valid approval with a matching hash submits (V1); an all-green
//! finding with no approval is refused (V2 — the "but everything
//! passed" case); an approval whose hash binds different bytes is
//! refused (A1); a replayed approval nonce is refused (A2). Every
//! refusal is typed. All doubles are scripted and labeled MOCK.

use crate::bounty::approve::sha256_hex;
use crate::bounty::{Approval, Clock, GateError, ManualClock, SubmissionGate};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-146";
/// Task name.
pub const NAME: &str = "operator-approval boundary on submission";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "valid_approval_and_hash_submits",
    "no_approval_refused",
    "hash_mismatch_refused",
    "replay_nonce_refused",
];

/// Fixture program the approvals bind to.
const PROGRAM_ID: &str = "prog-wave26";
/// Fixture scope version the approvals bind to.
const SCOPE_VERSION: u64 = 7;
/// Fixed scripted time (MOCK clock).
const NOW: u64 = 1_700_000_000;

/// A live operator approval for the fixture program and scope.
fn live_approval(nonce: u64) -> Approval {
    Approval {
        program_id: PROGRAM_ID.to_string(),
        scope_version: SCOPE_VERSION,
        granted_at: NOW - 60,
        expires_at: NOW + 3600,
        nonce,
        issuer: "operator".to_string(),
    }
}

/// Payload bytes the operator reviewed and approved (v1).
fn payload_v1() -> Vec<u8> {
    b"REPORT f000001\ntitle: xss in login\nseverity: high\n".to_vec()
}

/// Payload bytes the operator never saw (v2: severity quietly raised).
fn payload_v2() -> Vec<u8> {
    b"REPORT f000001\ntitle: xss in login\nseverity: critical\n".to_vec()
}

/// V1: valid approval + matching payload hash → submitted. The happy
/// path the boundary exists to protect.
fn case_valid_approval_and_hash_submits() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(NOW);
    let mut gate = SubmissionGate::new();
    let approval = live_approval(1001);
    let payload = payload_v1();
    let approved_hash = sha256_hex(&payload);
    let mut failures = Vec::new();
    match gate.submit(
        "f000001",
        true,
        &payload,
        &approved_hash,
        Some(&approval),
        PROGRAM_ID,
        SCOPE_VERSION,
        clock.now(),
    ) {
        Ok(sub) => {
            if sub.finding_id != "f000001" {
                failures.push(format!("submission finding_id wrong: {}", sub.finding_id));
            }
            if sub.payload_hash != approved_hash {
                failures.push("submission payload_hash != approved_hash".to_string());
            }
            if sub.approval_nonce != 1001 {
                failures.push(format!("submission nonce wrong: {}", sub.approval_nonce));
            }
            if gate.spent_nonce_count() != 1 {
                failures.push("nonce was not spent exactly once".to_string());
            }
        }
        Err(e) => failures.push(format!("valid approval + matching hash refused: {e:?}")),
    }
    let mut evidence = vec![
        format!("approved_hash={approved_hash}"),
        format!("nonce 1001 spent: {}", gate.spent_nonce_count()),
        "backend: ManualClock + fixture approval (MOCK)".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "submitted": failures.is_empty(),
            "spent_nonces": gate.spent_nonce_count(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: an all-green finding with no approval → `GateError::NoApproval`.
/// The "but everything passed" case: greenness is not authorization.
fn case_no_approval_refused() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(NOW);
    let mut gate = SubmissionGate::new();
    let payload = payload_v1();
    let approved_hash = sha256_hex(&payload);
    let mut failures = Vec::new();
    match gate.submit(
        "f000001",
        true,
        &payload,
        &approved_hash,
        None,
        PROGRAM_ID,
        SCOPE_VERSION,
        clock.now(),
    ) {
        Err(GateError::NoApproval) => {}
        Err(e) => failures.push(format!("wrong refusal: {e:?}, want NoApproval")),
        Ok(_) => failures.push("SUBMITTED with no approval — auto-submit!".to_string()),
    }
    if gate.spent_nonce_count() != 0 {
        failures.push("refused submission spent a nonce".to_string());
    }
    let mut evidence = vec![
        "refusal: GateError::NoApproval".to_string(),
        "backend: ManualClock + fixture approval (MOCK)".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "refusal": "NoApproval",
            "spent_nonces": gate.spent_nonce_count(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1: the approval binds the v1 bytes; submitting v2 bytes →
/// `GateError::HashMismatch`. The refused submission must not spend the
/// nonce — a failed attack must not consume the capability.
fn case_hash_mismatch_refused() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(NOW);
    let mut gate = SubmissionGate::new();
    let approval = live_approval(1002);
    let approved_hash = sha256_hex(&payload_v1());
    let v2_hash = sha256_hex(&payload_v2());
    let mut failures = Vec::new();
    match gate.submit(
        "f000001",
        true,
        &payload_v2(),
        &approved_hash,
        Some(&approval),
        PROGRAM_ID,
        SCOPE_VERSION,
        clock.now(),
    ) {
        Err(GateError::HashMismatch { expected, got }) => {
            if expected != approved_hash {
                failures.push("HashMismatch expected != approved v1 hash".to_string());
            }
            if got != v2_hash {
                failures.push("HashMismatch got != submitted v2 hash".to_string());
            }
        }
        Err(e) => failures.push(format!("wrong refusal: {e:?}, want HashMismatch")),
        Ok(_) => failures.push("SUBMITTED with mismatched payload bytes!".to_string()),
    }
    if gate.spent_nonce_count() != 0 {
        failures.push("refused submission spent the nonce".to_string());
    }
    let mut evidence = vec![
        format!("approved(v1)={approved_hash}"),
        format!("submitted(v2)={v2_hash}"),
        "refusal: GateError::HashMismatch; nonce unspent".to_string(),
        "backend: ManualClock + fixture approval (MOCK)".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "refusal": "HashMismatch",
            "spent_nonces": gate.spent_nonce_count(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2: replaying an approval nonce from an earlier submission →
/// `GateError::ReplayNonce`. Nonces are single-use; the gate keeps the
/// spent-nonce set.
fn case_replay_nonce_refused() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(NOW);
    let mut gate = SubmissionGate::new();
    let approval = live_approval(1003);
    let payload = payload_v1();
    let approved_hash = sha256_hex(&payload);
    let mut failures = Vec::new();
    let submit_once = |gate: &mut SubmissionGate| {
        gate.submit(
            "f000001",
            true,
            &payload,
            &approved_hash,
            Some(&approval),
            PROGRAM_ID,
            SCOPE_VERSION,
            clock.now(),
        )
    };
    if let Err(e) = submit_once(&mut gate) {
        failures.push(format!("first submission failed unexpectedly: {e:?}"));
    }
    match submit_once(&mut gate) {
        Err(GateError::ReplayNonce { nonce }) => {
            if nonce != 1003 {
                failures.push(format!("ReplayNonce named wrong nonce: {nonce}"));
            }
        }
        Err(e) => failures.push(format!("wrong refusal: {e:?}, want ReplayNonce")),
        Ok(_) => failures.push("REPLAYED nonce submitted twice!".to_string()),
    }
    if gate.spent_nonce_count() != 1 {
        failures.push(format!(
            "spent set wrong size: {}",
            gate.spent_nonce_count()
        ));
    }
    let mut evidence = vec![
        "first submission: ok; replay: GateError::ReplayNonce{nonce: 1003}".to_string(),
        format!("spent nonces: {}", gate.spent_nonce_count()),
        "backend: ManualClock + fixture approval (MOCK)".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "refusal": "ReplayNonce",
            "spent_nonces": gate.spent_nonce_count(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "valid_approval_and_hash_submits" => case_valid_approval_and_hash_submits(),
        "no_approval_refused" => case_no_approval_refused(),
        "hash_mismatch_refused" => case_hash_mismatch_refused(),
        "replay_nonce_refused" => case_replay_nonce_refused(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-146: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// no-auto-submit rule itself.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[1]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-146".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-146".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
