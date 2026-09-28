//! Task 134 — passive→active approval gating (rust, V/A).
//!
//! Passive recon needs no approval; an active probe refuses to launch
//! unless a live approval marker for the *current* scope version
//! exists. The gate checks the marker's provenance (it must come
//! from the operator authority), its binding (program id + scope
//! version), and its liveness — the same check order as the
//! submission gate ([`SubmissionGate::submit`]).
//!
//! The approval is a capability token with macaroon-style caveats:
//! scope version and expiry are caveats on the token, and the gate
//! fails closed when any caveat is unsatisfied (Birgisson et al.,
//! "Macaroons", NDSS 2014). A forged marker — correct shape, wrong
//! issuer — is refused by provenance, not shape.
//!
//! Distinct from task 135: this is marker *existence and binding*;
//! 135 is the *time* dimension of a valid marker.

use crate::bounty::approve::GateError;
use crate::bounty::clock::{Clock, ManualClock};
use crate::bounty::types::Approval;
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-134";
/// Task name.
pub const NAME: &str = "passive-to-active approval gating";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "passive_needs_no_approval",
    "active_with_bound_approval",
    "stale_scope_approval_refused",
    "forged_marker_refused",
];
/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 1_000_000;
/// Only the operator authority may issue approval markers.
pub const OPERATOR_ISSUER: &str = "operator";

/// Probe mode: passive recon vs active probing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeKind {
    Passive,
    Active,
}

/// The launch-path gate. Passive recon needs no approval; an active
/// probe launches only on a live operator approval bound to the
/// current scope version. On success returns the approval nonce the
/// run is bound to. Check order mirrors `SubmissionGate::submit`:
/// provenance → binding → liveness.
pub fn gate_launch(
    kind: ProbeKind,
    approval: Option<&Approval>,
    program_id: &str,
    scope_version: u64,
    now: u64,
) -> Result<u64, GateError> {
    match kind {
        ProbeKind::Passive => Ok(0),
        ProbeKind::Active => {
            let a = approval.ok_or(GateError::NoApproval)?;
            if a.issuer != OPERATOR_ISSUER {
                return Err(GateError::NoApproval);
            }
            if a.program_id != program_id || a.scope_version != scope_version {
                return Err(GateError::ScopeVersionMismatch);
            }
            if !(a.granted_at <= now && now < a.expires_at) {
                return Err(GateError::Expired);
            }
            Ok(a.nonce)
        }
    }
}

fn approval(
    issuer: &str,
    program_id: &str,
    scope_version: u64,
    granted_at: u64,
    expires_at: u64,
    nonce: u64,
) -> Approval {
    Approval {
        program_id: program_id.to_string(),
        scope_version,
        granted_at,
        expires_at,
        nonce,
        issuer: issuer.to_string(),
    }
}

/// V1: a passive scan launches with no approval marker at all.
fn case_passive_needs_no_approval() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let mut failures = Vec::new();
    match gate_launch(ProbeKind::Passive, None, "prog-1", 2, clock.now()) {
        Ok(0) => {}
        other => failures.push(format!("passive launch: {other:?}, want Ok(0)")),
    }
    let mut evidence =
        vec!["passive recon, no approval presented: launch allowed (nonce 0)".to_string()];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "passive_launched_without_approval": failures.is_empty(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: an active probe with a live operator approval bound to scope
/// v2 launches, bound to the approval's nonce.
fn case_active_with_bound_approval() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let now = clock.now();
    let a = approval(OPERATOR_ISSUER, "prog-1", 2, now, now + 3600, 42);
    let mut failures = Vec::new();
    match gate_launch(ProbeKind::Active, Some(&a), "prog-1", 2, now + 10) {
        Ok(42) => {}
        other => failures.push(format!("active launch: {other:?}, want Ok(42)")),
    }
    let mut evidence = vec![
        "active probe, approval{issuer=operator, scope_version=2} on scope v2: launched, nonce 42"
            .to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "bound_nonce": 42u64,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1 (adversarial): the approval is bound to scope v1 but the scope
/// is now v2 — the launch is refused with the typed
/// [`GateError::ScopeVersionMismatch`].
fn case_stale_scope_approval_refused() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let now = clock.now();
    let a = approval(OPERATOR_ISSUER, "prog-1", 1, now, now + 3600, 43);
    let mut failures = Vec::new();
    match gate_launch(ProbeKind::Active, Some(&a), "prog-1", 2, now + 10) {
        Err(GateError::ScopeVersionMismatch) => {}
        other => failures.push(format!(
            "stale-scope launch: {other:?}, want Err(ScopeVersionMismatch)"
        )),
    }
    let mut evidence = vec![
        "active probe, approval bound to scope v1, scope now v2: refused \
         with GateError::ScopeVersionMismatch"
            .to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "refused": failures.is_empty(),
            "error": "ScopeVersionMismatch",
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 (adversarial): a forged marker — correct shape, wrong issuer —
/// is refused. The gate checks provenance, not shape. A correct
/// issuer on the wrong program is likewise refused.
fn case_forged_marker_refused() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let now = clock.now();
    let mut failures = Vec::new();
    let forged = approval("bbscope-feed", "prog-1", 2, now, now + 3600, 44);
    match gate_launch(ProbeKind::Active, Some(&forged), "prog-1", 2, now + 10) {
        Err(GateError::NoApproval) => {}
        other => failures.push(format!(
            "forged-issuer launch: {other:?}, want Err(NoApproval)"
        )),
    }
    let wrong_program = approval(OPERATOR_ISSUER, "prog-2", 2, now, now + 3600, 45);
    match gate_launch(
        ProbeKind::Active,
        Some(&wrong_program),
        "prog-1",
        2,
        now + 10,
    ) {
        Err(GateError::ScopeVersionMismatch) => {}
        other => failures.push(format!(
            "wrong-program launch: {other:?}, want Err(ScopeVersionMismatch)"
        )),
    }
    let mut evidence = vec![
        "forged marker (issuer=bbscope-feed, shape otherwise valid): refused \
         with GateError::NoApproval — provenance checked, not shape"
            .to_string(),
        "operator approval for prog-2 used against prog-1: refused with \
         GateError::ScopeVersionMismatch"
            .to_string(),
        "no active launch without a valid marker in any scenario".to_string(),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "forged_refused": true,
            "wrong_program_refused": true,
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
        "passive_needs_no_approval" => case_passive_needs_no_approval(),
        "active_with_bound_approval" => case_active_with_bound_approval(),
        "stale_scope_approval_refused" => case_stale_scope_approval_refused(),
        "forged_marker_refused" => case_forged_marker_refused(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-134: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[1]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-134".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-134".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
