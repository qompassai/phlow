//! Task 140 — rate limits and testing windows (rust, validation +
//! adversarial).
//!
//! The program's rules of engagement are structural: `Scheduler::tick`
//! consults the `TestingWindow` and the token-bucket `RateLimit` before
//! emitting any `Launch`. Tick at 03:00 UTC inside the 02:00–04:00
//! window → launches proceed (V1); tick at 05:00 → `Hold`, queue
//! intact, zero launches (V2); the window closing with runs in flight
//! finishes them but launches nothing new (A1); scripted platform 429s
//! drive exponential backoff (1,2,4,… capped at 300s) with the request
//! rate bounded by `limit × elapsed + burst` (A2). Time is the clearly
//! labeled scripted double (MOCK): a shared cell clock the driver
//! advances.

use std::cell::Cell;
use std::rc::Rc;

use crate::bounty::{
    Clock, FakePlatform, Program, RateLimit, SchedAction, Scheduler, Target, TargetId, TargetKind,
    TargetQueue, TestingWindow,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-140";
/// Task name.
pub const NAME: &str = "rate limits and testing windows";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "window_open_launches",
    "window_closed_holds",
    "window_close_keeps_inflight",
    "rate_limit_429_backoff",
];

/// 02:00 UTC in epoch seconds (any date; the window uses time-of-day).
const T_0200: u64 = 2 * 3600;
/// 03:00 UTC — inside the window.
const T_0300: u64 = 3 * 3600;
/// 03:59 UTC — inside the window.
const T_0359: u64 = 3 * 3600 + 59 * 60;
/// 04:00 UTC — window closes (half-open end).
const T_0400: u64 = 4 * 3600;
/// 04:01 UTC — outside the window.
const T_0401: u64 = 4 * 3600 + 60;
/// 05:00 UTC — outside the window.
const T_0500: u64 = 5 * 3600;
/// Platform rate limit: 60 requests/minute.
const LIMIT_PER_MIN: u32 = 60;
/// Token-bucket burst.
const BURST: u32 = 5;
/// Backoff cap in seconds (scaffold-documented).
const BACKOFF_CAP_SECS: u64 = 300;
/// Bounded loop guard for the scripted 429 exchange.
const ATTEMPT_CAP: usize = 32;

/// Program under test: window 02:00–04:00 UTC, 60 req/min.
fn program_140() -> Program {
    Program {
        id: "p140".to_string(),
        name: "task-140 fixture program".to_string(),
        window: TestingWindow {
            start_min: 120,
            end_min: 240,
        },
        rate_limit: RateLimit {
            requests_per_minute: LIMIT_PER_MIN,
            burst: BURST,
        },
        allowed_suffixes: vec!["example.com".to_string()],
        max_cidr_prefix: 24,
    }
}

/// Shared scripted clock (MOCK). `Cell` (not `RefCell`): the driver
/// advances time between ticks; the scheduler only reads. No borrow
/// panics possible by construction.
#[derive(Clone, Debug, Default)]
struct SharedClock(Rc<Cell<u64>>);

impl SharedClock {
    fn new(t: u64) -> Self {
        SharedClock(Rc::new(Cell::new(t)))
    }

    fn set(&self, t: u64) {
        self.0.set(t);
    }

    fn advance(&self, secs: u64) {
        self.0.set(self.0.get().saturating_add(secs));
    }
}

impl Clock for SharedClock {
    fn now(&self) -> u64 {
        self.0.get()
    }
}

fn mk_target(i: usize) -> Target {
    Target {
        id: TargetId(format!("t{i:02}")),
        kind: TargetKind::Domain,
        value: format!("t{i:02}.example.com"),
    }
}

fn mk_sched(
    clock: &SharedClock,
    max_concurrent: u32,
) -> Result<Scheduler<SharedClock>, TaskDriverError> {
    Scheduler::new(program_140(), max_concurrent, clock.clone()).map_err(|detail| {
        TaskDriverError::Fixture {
            what: "scheduler".to_string(),
            detail: format!("task-140: scheduler construction failed: {detail}"),
        }
    })
}

fn hold_reason_is_window(action: &SchedAction) -> bool {
    matches!(action, SchedAction::Hold { reason } if reason == "testing window closed")
}

/// V1: tick at 03:00 (inside 02:00–04:00) → launches proceed.
fn case_window_open_launches() -> Result<CaseReport, TaskDriverError> {
    let clock = SharedClock::new(T_0300);
    let mut sched = mk_sched(&clock, 3)?;
    let mut queue = TargetQueue::new();
    for i in 0..5 {
        queue.push(mk_target(i));
    }
    let mut launched = 0usize;
    let mut drain = || queue.pop();
    for action in sched.tick(&mut drain) {
        match action {
            SchedAction::Launch(_) => {
                launched += 1;
                sched.note_run_finished();
            }
            other => {
                return Err(TaskDriverError::Arm {
                    arm: CASES[0].to_string(),
                    detail: format!("task-140: unexpected action inside window: {other:?}"),
                });
            }
        }
    }
    drop(drain);
    // Finish the first wave, launch the rest.
    let mut drain = || queue.pop();
    for action in sched.tick(&mut drain) {
        if let SchedAction::Launch(_) = action {
            launched += 1;
            sched.note_run_finished();
        }
    }
    let mut failures = Vec::new();
    if launched != 5 {
        failures.push(format!("launched {launched} != 5 inside the window"));
    }
    if !queue.is_empty() {
        failures.push("queue not drained inside the window".to_string());
    }
    let evidence = vec![
        "window 02:00-04:00 UTC, tick at 03:00".to_string(),
        format!("tick 1: 3 launches (K=3); tick 2: 2 launches; total {launched}"),
        "zero Hold actions inside the window".to_string(),
        "ManualClock-equivalent shared cell clock (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "launched": launched,
            "queue_empty": queue.is_empty(),
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    if !report.passed {
        report.failures.clone_from(&failures);
    }
    Ok(report)
}

/// V2: tick at 05:00 (outside the window) → Hold, queue intact, zero
/// launches.
fn case_window_closed_holds() -> Result<CaseReport, TaskDriverError> {
    let clock = SharedClock::new(T_0500);
    let mut sched = mk_sched(&clock, 3)?;
    let mut queue = TargetQueue::new();
    for i in 0..5 {
        queue.push(mk_target(i));
    }
    let mut drain = || queue.pop();
    let actions = sched.tick(&mut drain);
    drop(drain);
    let launches = actions
        .iter()
        .filter(|a| matches!(a, SchedAction::Launch(_)))
        .count();
    let mut failures = Vec::new();
    if launches != 0 {
        failures.push(format!("launched {launches} outside the window"));
    }
    if actions.len() != 1 || !hold_reason_is_window(&actions[0]) {
        failures.push(format!("expected single window Hold, got {actions:?}"));
    }
    if queue.len() != 5 {
        failures.push(format!("queue disturbed: len {}", queue.len()));
    }
    if sched.in_flight() != 0 {
        failures.push("in_flight moved outside the window".to_string());
    }
    let evidence = vec![
        "window 02:00-04:00 UTC, tick at 05:00".to_string(),
        format!("actions: {actions:?}"),
        "zero launches; queue intact (5/5); in_flight 0".to_string(),
        "shared cell clock (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "launches": launches,
            "holds": actions.len(),
            "queue_len": queue.len(),
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    if !report.passed {
        report.failures.clone_from(&failures);
    }
    Ok(report)
}

/// A1 (adversarial): the window closes at 04:00 with 2 runs in flight.
/// In-flight runs finish; zero new launches happen after 04:00.
fn case_window_close_keeps_inflight() -> Result<CaseReport, TaskDriverError> {
    let clock = SharedClock::new(T_0359);
    let mut sched = mk_sched(&clock, 2)?;
    let mut queue = TargetQueue::new();
    for i in 0..4 {
        queue.push(mk_target(i));
    }
    let mut launch_times: Vec<(String, u64)> = Vec::new();
    let mut drain = || queue.pop();
    for action in sched.tick(&mut drain) {
        if let SchedAction::Launch(t) = action {
            launch_times.push((t.id.0.clone(), clock.now()));
        }
    }
    drop(drain);
    // Window closes; the two in-flight runs finish on the driver's side.
    clock.set(T_0400);
    let mut drain = || queue.pop();
    let actions_at_close = sched.tick(&mut drain);
    drop(drain);
    sched.note_run_finished();
    sched.note_run_finished();
    clock.set(T_0401);
    let mut drain = || queue.pop();
    let actions_after = sched.tick(&mut drain);
    drop(drain);
    let launches_after_close = launch_times.iter().filter(|(_, t)| *t >= T_0400).count();
    let mut failures = Vec::new();
    if launch_times.len() != 2 {
        failures.push(format!("launched {} != 2 before close", launch_times.len()));
    }
    if launches_after_close != 0 {
        failures.push(format!("{launches_after_close} launches at/after 04:00"));
    }
    for (at, actions) in [("close", &actions_at_close), ("after", &actions_after)] {
        if actions.len() != 1 || !hold_reason_is_window(&actions[0]) {
            failures.push(format!(
                "tick {at}: expected single window Hold, got {actions:?}"
            ));
        }
    }
    if queue.len() != 2 {
        failures.push(format!("queue not intact: len {}", queue.len()));
    }
    if sched.in_flight() != 0 {
        failures.push("in_flight != 0 after finishing".to_string());
    }
    let evidence = vec![
        "tick at 03:59: 2 launches (in flight across the close)".to_string(),
        "tick at 04:00 and 04:01: Hold { reason: \"testing window closed\" }".to_string(),
        format!("launches at/after 04:00: {launches_after_close} (zero)"),
        "queue intact: 2/2 unlaunched targets still queued".to_string(),
        "in-flight runs finished via note_run_finished (driver-side completion)".to_string(),
        "shared cell clock (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "launched_before_close": launch_times.len(),
            "launches_after_close": launches_after_close,
            "queue_len": queue.len(),
            "in_flight": sched.in_flight(),
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    if !report.passed {
        report.failures.clone_from(&failures);
    }
    Ok(report)
}

/// A2 (adversarial): the platform answers 429. The scheduler backs off
/// exponentially (1,2,4,… capped at 300s); attempts stay within
/// `limit × elapsed + burst` — no retry storm.
fn case_rate_limit_429_backoff() -> Result<CaseReport, TaskDriverError> {
    let clock = SharedClock::new(T_0300);
    let mut sched = mk_sched(&clock, 1)?;
    let mut queue = TargetQueue::new();
    queue.push(mk_target(0));
    let mut platform = FakePlatform::new();
    // NOTE: FakePlatform::fail_next_submit is a one-shot boolean, so it
    // is re-armed immediately before each of the first three
    // submissions below (explicit bounded refusal counter).
    let mut attempts: Vec<u64> = Vec::new();
    let mut backoffs: Vec<u64> = Vec::new();
    let t0 = clock.now();
    loop {
        if attempts.len() > ATTEMPT_CAP {
            return Err(TaskDriverError::Arm {
                arm: CASES[3].to_string(),
                detail: "task-140: 429 exchange exceeded attempt cap (retry storm)".to_string(),
            });
        }
        let mut drain = || queue.pop();
        let actions = sched.tick(&mut drain);
        drop(drain);
        match actions.first() {
            Some(SchedAction::Backoff { secs }) => {
                backoffs.push(*secs);
                clock.advance(1);
            }
            Some(SchedAction::Hold { reason }) => {
                return Err(TaskDriverError::Arm {
                    arm: CASES[3].to_string(),
                    detail: format!("task-140: unexpected Hold during 429 exchange: {reason}"),
                });
            }
            Some(SchedAction::Launch(target)) => {
                let now = clock.now();
                attempts.push(now);
                // The platform refused the launch: return the target and
                // release the phantom slot (driver-level protocol).
                queue.push(target.clone());
                sched.note_run_finished();
                if platform.submit_calls() < 3 {
                    platform.fail_next_submit();
                }
                match platform.submit(b"probe-request") {
                    Err(429) => sched.note_platform_429(),
                    Ok(_) => {
                        sched.note_platform_success();
                        break;
                    }
                    Err(code) => {
                        return Err(TaskDriverError::Arm {
                            arm: CASES[3].to_string(),
                            detail: format!("task-140: platform answered {code}, want 429/ok"),
                        });
                    }
                }
                clock.advance(1);
            }
            None => {
                return Err(TaskDriverError::Arm {
                    arm: CASES[3].to_string(),
                    detail: "task-140: tick emitted no actions during 429 exchange".to_string(),
                });
            }
        }
    }
    let mut failures = Vec::new();
    // Exponential gaps: attempts at t0, t0+1, t0+3, t0+7.
    let gaps: Vec<u64> = attempts
        .windows(2)
        .map(|w| w[1].saturating_sub(w[0]))
        .collect();
    if gaps != vec![1, 2, 4] {
        failures.push(format!(
            "attempt gaps {gaps:?} != [1, 2, 4]: not exponential"
        ));
    }
    let elapsed = attempts.last().copied().unwrap_or(t0).saturating_sub(t0);
    let bound = u64::from(LIMIT_PER_MIN) * elapsed / 60 + u64::from(BURST);
    if attempts.len() as u64 > bound {
        failures.push(format!(
            "{} attempts in {elapsed}s exceed limit*elapsed+burst = {bound}",
            attempts.len()
        ));
    }
    if platform.submit_calls() != attempts.len() as u64 {
        failures.push("platform call count disagrees with attempt log".to_string());
    }
    if backoffs.is_empty() {
        failures.push("no Backoff actions observed".to_string());
    }
    // The cap, unit-checked: 1,2,4,…,256,300,300 on a fixed clock.
    let clock2 = SharedClock::new(T_0200);
    let mut sched2 = mk_sched(&clock2, 1)?;
    let mut deltas = Vec::new();
    for _ in 0..12 {
        sched2.note_platform_429();
        // Read the backoff through a tick with an empty drain: the
        // scaffold answers Backoff { secs } while backing off.
        let mut no_targets = || None;
        let secs = match sched2.tick(&mut no_targets).first() {
            Some(SchedAction::Backoff { secs }) => *secs,
            other => {
                return Err(TaskDriverError::Arm {
                    arm: CASES[3].to_string(),
                    detail: format!("task-140: expected Backoff in cap check, got {other:?}"),
                });
            }
        };
        deltas.push(secs);
    }
    let want: Vec<u64> = vec![
        1,
        2,
        4,
        8,
        16,
        32,
        64,
        128,
        256,
        BACKOFF_CAP_SECS,
        BACKOFF_CAP_SECS,
        BACKOFF_CAP_SECS,
    ];
    if deltas != want {
        failures.push(format!(
            "backoff schedule {deltas:?} != {want:?}: cap broken"
        ));
    }
    let evidence = vec![
        "platform scripted: 3x 429 then Ok (FakePlatform::fail_next_submit)".to_string(),
        format!(
            "attempts at {:?} (gaps {gaps:?}: exponential 1,2,4)",
            attempts
        ),
        format!("Backoff secs observed: {backoffs:?}"),
        format!(
            "{} attempts in {elapsed}s <= {LIMIT_PER_MIN}*{elapsed}/60+{BURST} = {bound}: no storm",
            attempts.len()
        ),
        format!("backoff cap unit check: {deltas:?}"),
        "shared cell clock (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "attempts": attempts.len(),
            "gaps": gaps,
            "elapsed_secs": elapsed,
            "bound": bound,
            "backoff_cap_schedule": deltas,
            "backend": "scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    if !report.passed {
        report.failures.clone_from(&failures);
    }
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "window_open_launches" => case_window_open_launches(),
        "window_closed_holds" => case_window_closed_holds(),
        "window_close_keeps_inflight" => case_window_close_keeps_inflight(),
        "rate_limit_429_backoff" => case_rate_limit_429_backoff(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-140: unknown case '{case}'"),
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
            where_: "task-140".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-140".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
