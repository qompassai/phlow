//! Task 133 — scope revocation mid-cycle (rust, V).
//!
//! A target that leaves scope mid-cycle must not be probed. This is
//! the *policy* seam (task 132 computes the diff; task 137 owns the
//! cancellation mechanics): on each snapshot transition the driver
//! diffs old→new, drops removed targets from the queue, and cancels
//! their non-terminal runs with the `ScopeRevoked` reason. Queueing
//! is structural — a target is queueable only when the *latest*
//! snapshot covers it, so a re-added target becomes queueable again
//! while a revoked one can never be launched.
//!
//! Out-of-scope testing is a program violation, so the refusal is
//! structural (the queue gate), not advisory. Snapshots are served
//! by the scripted [`ScriptedFeed`] (MOCK).

use crate::bounty::diff::diff_scope;
use crate::bounty::feed::{FeedError, ScopeFeed, ScriptedFeed, snapshot, target};
use crate::bounty::store::{RunLedger, TargetQueue};
use crate::bounty::types::{Run, RunState, ScopeSnapshot, Target, TargetId, TargetKind};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-133";
/// Task name.
pub const NAME: &str = "scope revocation mid-cycle";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "revoked_queued_target_dropped",
    "revoked_running_run_cancelled",
    "unknown_revocation_noop",
    "readded_target_queueable",
];
/// Cancel reason recorded on runs killed by a scope revocation.
pub const SCOPE_REVOKED_REASON: &str = "ScopeRevoked";

/// Outcome of revoking one target by id.
#[derive(Debug, PartialEq, Eq)]
pub enum RevocationOutcome {
    /// The id was unknown: queue and ledger untouched.
    UnknownTarget,
    /// The id was revoked: (dropped from queue, runs cancelled).
    Applied {
        dropped_from_queue: bool,
        runs_cancelled: u64,
    },
}

/// Typed refusal when queueing a target the latest snapshot does not
/// cover.
#[derive(Debug, PartialEq, Eq)]
pub enum QueueRefusal {
    NotInScope { id: String },
}

/// Revoke one target: drop it from the queue and cancel its
/// non-terminal runs with the [`SCOPE_REVOKED_REASON`] reason.
fn revoke_target(
    queue: &mut TargetQueue,
    ledger: &mut RunLedger,
    id: &TargetId,
) -> RevocationOutcome {
    let dropped_from_queue = queue.cancel(id);
    let run_ids: Vec<String> = ledger
        .runs()
        .iter()
        .filter(|r| &r.target_id == id && matches!(r.state, RunState::Queued | RunState::Running))
        .map(|r| r.id.clone())
        .collect();
    for run_id in &run_ids {
        ledger.set_state(
            run_id,
            RunState::Cancelled,
            Some(SCOPE_REVOKED_REASON.to_string()),
        );
    }
    let runs_cancelled = run_ids.len() as u64;
    if !dropped_from_queue && runs_cancelled == 0 {
        RevocationOutcome::UnknownTarget
    } else {
        RevocationOutcome::Applied {
            dropped_from_queue,
            runs_cancelled,
        }
    }
}

/// Apply a snapshot transition: diff old→new, revoke every removed
/// target. Returns (removed target id, outcome) pairs.
fn apply_scope_transition(
    queue: &mut TargetQueue,
    ledger: &mut RunLedger,
    old: &ScopeSnapshot,
    new: &ScopeSnapshot,
) -> Vec<(String, RevocationOutcome)> {
    diff_scope(old, new)
        .removed
        .iter()
        .map(|t| {
            let outcome = revoke_target(queue, ledger, &t.id);
            (t.id.0.clone(), outcome)
        })
        .collect()
}

/// Queue a target only when the latest snapshot covers it — the
/// "latest snapshot wins" policy, enforced structurally at the queue
/// mouth.
fn try_enqueue(
    queue: &mut TargetQueue,
    latest: &ScopeSnapshot,
    t: Target,
) -> Result<(), QueueRefusal> {
    if latest.targets.iter().any(|x| x.id == t.id) {
        queue.push(t);
        Ok(())
    } else {
        Err(QueueRefusal::NotInScope { id: t.id.0.clone() })
    }
}

fn snap_v1(at: u64) -> ScopeSnapshot {
    snapshot(
        1,
        at,
        vec![
            target("a", TargetKind::Domain, "a.example.com"),
            target("b", TargetKind::Domain, "b.example.com"),
            target("c", TargetKind::Domain, "c.example.com"),
        ],
    )
}

/// v2 drops b.
fn snap_v2(at: u64) -> ScopeSnapshot {
    snapshot(
        2,
        at,
        vec![
            target("a", TargetKind::Domain, "a.example.com"),
            target("c", TargetKind::Domain, "c.example.com"),
        ],
    )
}

/// v3 re-adds b.
fn snap_v3(at: u64) -> ScopeSnapshot {
    snapshot(
        3,
        at,
        vec![
            target("a", TargetKind::Domain, "a.example.com"),
            target("b", TargetKind::Domain, "b.example.com"),
            target("c", TargetKind::Domain, "c.example.com"),
        ],
    )
}

/// Serve the three scripted snapshots (v1, v2, v3) from the
/// [`ScriptedFeed`] (MOCK).
fn scripted_snaps() -> Result<(ScopeSnapshot, ScopeSnapshot, ScopeSnapshot), TaskDriverError> {
    let mut feed = ScriptedFeed::new(vec![
        snap_v1(1_000_000),
        snap_v2(1_000_060),
        snap_v3(1_000_120),
    ]);
    let err = |e: FeedError| TaskDriverError::Fixture {
        what: "scripted-feed".to_string(),
        detail: format!("task-133: scripted feed failed: {e:?}"),
    };
    Ok((
        feed.poll().map_err(err)?,
        feed.poll().map_err(err)?,
        feed.poll().map_err(err)?,
    ))
}

/// Mid-cycle state: queue {a,b,c} with a and c launched (Running runs
/// recorded in the ledger); b still queued, never launched.
fn mid_cycle_fixture() -> (TargetQueue, RunLedger) {
    let mut queue = TargetQueue::new();
    queue.push(target("a", TargetKind::Domain, "a.example.com"));
    queue.push(target("b", TargetKind::Domain, "b.example.com"));
    queue.push(target("c", TargetKind::Domain, "c.example.com"));
    let mut ledger = RunLedger::new();
    let a = queue.pop().expect("task-133: fixture queue must hold a");
    let b = queue.pop().expect("task-133: fixture queue must hold b");
    let c = queue.pop().expect("task-133: fixture queue must hold c");
    queue.push(b);
    for (run_id, t) in [("run-a1", a), ("run-c1", c)] {
        ledger.record(Run {
            id: run_id.to_string(),
            target_id: t.id,
            state: RunState::Running,
            approval_nonce: 0,
            cancel_reason: None,
        });
    }
    (queue, ledger)
}

fn b_id() -> TargetId {
    TargetId("b".to_string())
}

/// V1: v2 revokes b while b is queued. b is dropped from the queue
/// and the ledger shows zero runs for b — it was never launched.
fn case_revoked_queued_target_dropped() -> Result<CaseReport, TaskDriverError> {
    let (v1, v2, _) = scripted_snaps()?;
    let (mut queue, mut ledger) = mid_cycle_fixture();
    let outcomes = apply_scope_transition(&mut queue, &mut ledger, &v1, &v2);
    let mut failures = Vec::new();
    let b_outcome = outcomes.iter().find(|(id, _)| id == "b").map(|(_, o)| o);
    match b_outcome {
        Some(RevocationOutcome::Applied {
            dropped_from_queue: true,
            runs_cancelled: 0,
        }) => {}
        other => failures.push(format!(
            "revocation outcome for b: {other:?}, want Applied{{dropped, 0}}"
        )),
    }
    if queue.contains(&b_id()) {
        failures.push("b still in queue after revocation".to_string());
    }
    let b_runs = ledger
        .runs()
        .iter()
        .filter(|r| r.target_id == b_id())
        .count();
    if b_runs != 0 {
        failures.push(format!("ledger shows {b_runs} runs for revoked b, want 0"));
    }
    // a's run is untouched: revocation of b must not disturb it.
    let a_running = ledger
        .runs()
        .iter()
        .any(|r| r.target_id.0 == "a" && r.state == RunState::Running);
    if !a_running {
        failures.push("a's run disturbed by b's revocation".to_string());
    }
    let mut evidence = vec![
        format!("v1 -> v2 revocation outcomes: {outcomes:?}"),
        format!("b in queue after revocation: {}", queue.contains(&b_id())),
        format!("ledger runs for b: {b_runs} (never launched)"),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "b_revoked_from_queue": !queue.contains(&b_id()),
            "b_runs": b_runs,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: v2 revokes b while c's run is Running. c's run transitions
/// Running → Cancelled with the `ScopeRevoked` reason recorded.
fn case_revoked_running_run_cancelled() -> Result<CaseReport, TaskDriverError> {
    let (v1, _, _) = scripted_snaps()?;
    let (mut queue, mut ledger) = mid_cycle_fixture();
    // c's revocation: reuse the transition on a snapshot that drops c
    // instead of b, so the Running-run path is exercised. (The v2
    // fixture revokes b; b's queued-drop path is V1's case.)
    let v2_no_c = snapshot(
        2,
        1_000_060,
        vec![
            target("a", TargetKind::Domain, "a.example.com"),
            target("b", TargetKind::Domain, "b.example.com"),
        ],
    );
    let outcomes = apply_scope_transition(&mut queue, &mut ledger, &v1, &v2_no_c);
    let mut failures = Vec::new();
    let c_run = ledger.runs().iter().find(|r| r.target_id.0 == "c").cloned();
    match c_run.as_ref() {
        Some(r)
            if r.state == RunState::Cancelled
                && r.cancel_reason.as_deref() == Some(SCOPE_REVOKED_REASON) => {}
        other => failures.push(format!(
            "c's run after revocation: {other:?}, want Cancelled with reason {SCOPE_REVOKED_REASON}"
        )),
    }
    let c_outcome = outcomes.iter().find(|(id, _)| id == "c").map(|(_, o)| o);
    if c_outcome
        != Some(&RevocationOutcome::Applied {
            dropped_from_queue: false,
            runs_cancelled: 1,
        })
    {
        failures.push(format!(
            "revocation outcome for c: {c_outcome:?}, want Applied{{not queued, 1 cancelled}}"
        ));
    }
    let mut evidence = vec![
        format!("v1 -> v2(no-c) revocation outcomes: {outcomes:?}"),
        format!(
            "c's run state: {:?}, reason: {:?}",
            c_run.as_ref().map(|r| &r.state),
            c_run.as_ref().and_then(|r| r.cancel_reason.clone())
        ),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "c_run_state": format!("{:?}", c_run.as_ref().map(|r| &r.state)),
            "cancel_reason": c_run.as_ref().and_then(|r| r.cancel_reason.clone()),
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1 (adversarial): a revocation naming an unknown target is a typed
/// no-op — queue and ledger untouched.
fn case_unknown_revocation_noop() -> Result<CaseReport, TaskDriverError> {
    let (mut queue, mut ledger) = mid_cycle_fixture();
    let queue_len = queue.len();
    let ledger_len = ledger.runs().len();
    let outcome = revoke_target(&mut queue, &mut ledger, &TargetId("ghost".to_string()));
    let mut failures = Vec::new();
    if outcome != RevocationOutcome::UnknownTarget {
        failures.push(format!(
            "unknown-target revocation: {outcome:?}, want UnknownTarget"
        ));
    }
    if queue.len() != queue_len {
        failures.push("queue changed by unknown-target revocation".to_string());
    }
    if ledger.runs().len() != ledger_len {
        failures.push("ledger changed by unknown-target revocation".to_string());
    }
    let mut evidence = vec![format!(
        "revoke 'ghost': {outcome:?}; queue len {queue_len} -> {}, ledger runs {ledger_len} -> {}",
        queue.len(),
        ledger.runs().len()
    )];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "outcome": format!("{outcome:?}"),
            "queue_untouched": queue.len() == queue_len,
            "ledger_untouched": ledger.runs().len() == ledger_len,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 (adversarial): v3 re-adds b. Under v2 b is not queueable
/// (typed refusal); under v3 — the latest snapshot wins — it is.
fn case_readded_target_queueable() -> Result<CaseReport, TaskDriverError> {
    let (_, v2, v3) = scripted_snaps()?;
    let mut queue = TargetQueue::new();
    let mut failures = Vec::new();
    let b = || target("b", TargetKind::Domain, "b.example.com");
    match try_enqueue(&mut queue, &v2, b()) {
        Err(QueueRefusal::NotInScope { id }) if id == "b" => {}
        other => failures.push(format!("enqueue b under v2: {other:?}, want NotInScope")),
    }
    if !queue.is_empty() {
        failures.push("b entered the queue under v2".to_string());
    }
    match try_enqueue(&mut queue, &v3, b()) {
        Ok(()) => {}
        Err(e) => failures.push(format!("enqueue b under v3 refused: {e:?}")),
    }
    if !queue.contains(&b_id()) {
        failures.push("b not queueable after v3 re-added it".to_string());
    }
    let mut evidence = vec![
        "enqueue b under v2 (revoked): Err(NotInScope) — structural refusal".to_string(),
        format!(
            "enqueue b under v3 (re-added): ok; queue holds b: {}",
            queue.contains(&b_id())
        ),
    ];
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "refused_under_v2": true,
            "queueable_under_v3": queue.contains(&b_id()),
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
        "revoked_queued_target_dropped" => case_revoked_queued_target_dropped(),
        "revoked_running_run_cancelled" => case_revoked_running_run_cancelled(),
        "unknown_revocation_noop" => case_unknown_revocation_noop(),
        "readded_target_queueable" => case_readded_target_queueable(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-133: unknown case '{case}'"),
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
            where_: "task-133".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-133".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
