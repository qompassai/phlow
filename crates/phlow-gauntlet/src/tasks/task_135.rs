//! Task 135 — approval expiry (rust, V/A).
//!
//! A stale approval authorizes nothing new. The expiry is a
//! macaroon-style time caveat (`time < expires_at`, cf. Birgisson et
//! al., "Macaroons", NDSS 2014): the gate fails closed at the expiry
//! second. In-flight runs may finish — the gate is launch-only — but
//! every *new* launch after expiry is refused with the typed
//! [`GateError::Expired`]. A backdated approval (`granted_at` in the
//! future) is rejected at issuance and never enters the store.
//!
//! Distinct from task 134: this is the *time* dimension of a valid
//! marker; 134 is marker *existence and binding*. The launch gate is
//! deliberately duplicated from task_134's (drivers stay disjoint;
//! the scaffold owns no launch gate).

use crate::bounty::approve::GateError;
use crate::bounty::clock::{Clock, ManualClock};
use crate::bounty::store::RunLedger;
use crate::bounty::types::{Approval, Run, RunState};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-135";
/// Task name.
pub const NAME: &str = "approval expiry";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "launch_before_expiry",
    "launch_after_expiry",
    "inflight_finishes_new_blocked",
    "backdated_rejected_at_issuance",
];
/// Approval time-to-live (seconds). Expiry = granted_at + this.
pub const APPROVAL_TTL_SECS: u64 = 3600;
/// Scripted epoch for the [`ManualClock`].
pub const CLOCK_START: u64 = 2_000_000;

/// Probe mode: passive recon vs active probing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeKind {
    Passive,
    Active,
}

/// The launch-path gate (same contract as task_134's: provenance →
/// binding → liveness). On success returns the approval nonce the run
/// is bound to.
fn gate_launch(
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
            if a.issuer != "operator" {
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

/// Typed issuance refusal: an approval that is not live yet (or has
/// no live interval at all) must never enter the store.
#[derive(Debug, PartialEq, Eq)]
pub enum IssuanceError {
    /// `granted_at` is in the future: the approval is not live yet.
    NotYetGranted { granted_at: u64, now: u64 },
    /// `expires_at <= granted_at`: the approval is never live.
    NeverLive { granted_at: u64, expires_at: u64 },
}

/// Store an operator approval after issuance checks. Backdated or
/// zero/negative-TTL approvals are rejected and never stored.
fn issue_approval(store: &mut Vec<Approval>, a: Approval, now: u64) -> Result<(), IssuanceError> {
    if a.granted_at > now {
        return Err(IssuanceError::NotYetGranted {
            granted_at: a.granted_at,
            now,
        });
    }
    if a.expires_at <= a.granted_at {
        return Err(IssuanceError::NeverLive {
            granted_at: a.granted_at,
            expires_at: a.expires_at,
        });
    }
    store.push(a);
    Ok(())
}

fn approval(granted_at: u64, expires_at: u64, nonce: u64) -> Approval {
    Approval {
        program_id: "prog-1".to_string(),
        scope_version: 2,
        granted_at,
        expires_at,
        nonce,
        issuer: "operator".to_string(),
    }
}

/// V1: a launch one second before expiry is allowed.
fn case_launch_before_expiry() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(CLOCK_START);
    clock.set(CLOCK_START + APPROVAL_TTL_SECS - 1);
    let a = approval(CLOCK_START, CLOCK_START + APPROVAL_TTL_SECS, 7);
    let mut failures = Vec::new();
    match gate_launch(ProbeKind::Active, Some(&a), "prog-1", 2, clock.now()) {
        Ok(7) => {}
        other => failures.push(format!("launch at T=expiry-1: {other:?}, want Ok(7)")),
    }
    let mut evidence = vec![format!(
        "approval expires at T={}; launch at T={} allowed, bound to nonce 7",
        CLOCK_START + APPROVAL_TTL_SECS,
        clock.now()
    )];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "launched_at": clock.now(),
            "expires_at": CLOCK_START + APPROVAL_TTL_SECS,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: the boundary is exact at the expiry second — launch at
/// T=expiry and T=expiry+1 are both refused with
/// [`GateError::Expired`].
fn case_launch_after_expiry() -> Result<CaseReport, TaskDriverError> {
    let a = approval(CLOCK_START, CLOCK_START + APPROVAL_TTL_SECS, 7);
    let mut failures = Vec::new();
    for (label, at) in [
        ("T=expiry", CLOCK_START + APPROVAL_TTL_SECS),
        ("T=expiry+1", CLOCK_START + APPROVAL_TTL_SECS + 1),
    ] {
        match gate_launch(ProbeKind::Active, Some(&a), "prog-1", 2, at) {
            Err(GateError::Expired) => {}
            other => failures.push(format!("launch at {label}: {other:?}, want Err(Expired)")),
        }
    }
    let mut evidence = vec![format!(
        "boundary exact at the expiry second: launch at T={} and T={} \
             both refused with GateError::Expired",
        CLOCK_START + APPROVAL_TTL_SECS,
        CLOCK_START + APPROVAL_TTL_SECS + 1
    )];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "expires_at": CLOCK_START + APPROVAL_TTL_SECS,
            "refused_at_boundary": true,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1 (adversarial): a run launched at T=expiry-1 is still running at
/// T=expiry+100. It may finish (the gate is launch-only) — but a new
/// launch at T=expiry+100 is refused. The ledger distinguishes the
/// two.
fn case_inflight_finishes_new_blocked() -> Result<CaseReport, TaskDriverError> {
    let mut clock = ManualClock::new(CLOCK_START);
    let a = approval(CLOCK_START, CLOCK_START + APPROVAL_TTL_SECS, 7);
    let mut ledger = RunLedger::new();
    let mut failures = Vec::new();
    // Launch just before expiry.
    clock.set(CLOCK_START + APPROVAL_TTL_SECS - 1);
    let nonce =
        gate_launch(ProbeKind::Active, Some(&a), "prog-1", 2, clock.now()).map_err(|e| {
            TaskDriverError::Arm {
                arm: "pre-expiry-launch".to_string(),
                detail: format!("task-135: lawful launch refused: {e:?}"),
            }
        })?;
    ledger.record(Run {
        id: "run-1".to_string(),
        target_id: crate::bounty::types::TargetId("t1".to_string()),
        state: RunState::Running,
        approval_nonce: nonce,
        cancel_reason: None,
    });
    // Past expiry the in-flight run may finish: completing it is not
    // a new launch, so the gate does not block it.
    clock.set(CLOCK_START + APPROVAL_TTL_SECS + 100);
    ledger.set_state("run-1", RunState::Finished, None);
    if ledger.count_in_state(RunState::Finished) != 1 {
        failures.push("in-flight run could not finish after expiry".to_string());
    }
    // But a NEW launch at the same instant is refused.
    match gate_launch(ProbeKind::Active, Some(&a), "prog-1", 2, clock.now()) {
        Err(GateError::Expired) => {}
        other => failures.push(format!(
            "new launch at T=expiry+100: {other:?}, want Err(Expired)"
        )),
    }
    if ledger.runs().len() != 1 {
        failures.push(format!(
            "ledger holds {} runs, want exactly 1 (no new launch recorded)",
            ledger.runs().len()
        ));
    }
    let mut evidence = vec![
        "run launched at T=expiry-1, still Running at T=expiry+100: finished normally (gate is launch-only)".to_string(),
        "new launch at T=expiry+100: refused with GateError::Expired".to_string(),
        format!(
            "ledger: {} run, {} Finished — completion vs new-launch distinguished",
            ledger.runs().len(),
            ledger.count_in_state(RunState::Finished)
        ),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "inflight_finished": ledger.count_in_state(RunState::Finished) == 1,
            "new_launch_refused": true,
            "ledger_runs": ledger.runs().len(),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 (adversarial): a backdated approval (`granted_at` in the
/// future) is rejected at issuance and never enters the store. A
/// zero-TTL approval is likewise rejected.
fn case_backdated_rejected_at_issuance() -> Result<CaseReport, TaskDriverError> {
    let clock = ManualClock::new(CLOCK_START);
    let now = clock.now();
    let mut store: Vec<Approval> = Vec::new();
    let mut failures = Vec::new();
    let backdated = approval(now + 7200, now + 10800, 8);
    match issue_approval(&mut store, backdated, now) {
        Err(IssuanceError::NotYetGranted { granted_at, now: n })
            if granted_at == now + 7200 && n == now => {}
        other => failures.push(format!(
            "backdated issuance: {other:?}, want Err(NotYetGranted)"
        )),
    }
    let zero_ttl = approval(now, now, 9);
    match issue_approval(&mut store, zero_ttl, now) {
        Err(IssuanceError::NeverLive { .. }) => {}
        other => failures.push(format!("zero-TTL issuance: {other:?}, want Err(NeverLive)")),
    }
    if !store.is_empty() {
        failures.push(format!(
            "store holds {} approvals after rejected issuance, want 0",
            store.len()
        ));
    }
    let mut evidence = vec![
        "backdated approval (granted_at = now+7200): rejected at issuance with \
         IssuanceError::NotYetGranted — never stored"
            .to_string(),
        "zero-TTL approval (expires_at == granted_at): rejected with \
         IssuanceError::NeverLive — never stored"
            .to_string(),
        format!("approval store size after rejections: {}", store.len()),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "store_size": store.len(),
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
        "launch_before_expiry" => case_launch_before_expiry(),
        "launch_after_expiry" => case_launch_after_expiry(),
        "inflight_finishes_new_blocked" => case_inflight_finishes_new_blocked(),
        "backdated_rejected_at_issuance" => case_backdated_rejected_at_issuance(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-135: unknown case '{case}'"),
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
            where_: "task-135".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-135".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
