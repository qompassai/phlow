//! Task 136 — target queueing with bounded concurrency (rust, validation).
//!
//! `Scheduler::tick` with `max_concurrent = K` is the structural bound:
//! the tick loop only emits `Launch` while `in_flight < K`, so the bound
//! cannot drift no matter what the queue does. This task pins it: 10
//! targets at K=3 reach peak exactly 3 and all complete (V1); K=1 is
//! strictly serial (V2); a crashed worker (no heartbeat past the
//! timeout) releases its slot and is marked `Failed` while the queue
//! keeps draining (A1); K=0 is refused at construction, fail closed
//! (A2). All evidence comes from the clearly labeled scripted double
//! (MOCK): `ManualClock` plus a scripted probe-cycle simulator with
//! fixed probe durations.

use std::collections::{HashMap, HashSet};

use crate::bounty::{
    ManualClock, Program, RateLimit, Run, RunLedger, RunState, SchedAction, Scheduler, Target,
    TargetId, TargetKind, TargetQueue, TestingWindow,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-136";
/// Task name.
pub const NAME: &str = "target queueing with bounded concurrency";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "peak_bound_k3",
    "serial_at_k1",
    "crash_releases_slot",
    "k0_refused",
];

/// Ticks one scripted probe takes to finish.
pub const PROBE_TICKS: u64 = 2;
/// Heartbeat timeout in ticks for the crash arm.
pub const HEARTBEAT_TIMEOUT_TICKS: u64 = 3;
/// Targets in the main scenario.
pub const TARGET_COUNT: usize = 10;
/// Hard cap on simulated ticks; hitting it is an apparatus failure.
pub const TICK_CAP: u64 = 500;

fn mk_target(i: usize) -> Target {
    Target {
        id: TargetId(format!("t{i:02}")),
        kind: TargetKind::Domain,
        value: format!("t{i:02}.example.com"),
    }
}

/// Program with the testing window always open and the platform rate
/// limit effectively infinite: the only structural bound under test is
/// `max_concurrent`.
fn open_program() -> Program {
    Program {
        id: "p136".to_string(),
        name: "task-136 fixture program".to_string(),
        window: TestingWindow {
            start_min: 0,
            end_min: 1440,
        },
        rate_limit: RateLimit {
            requests_per_minute: 3600,
            burst: 1000,
        },
        allowed_suffixes: vec!["example.com".to_string()],
        max_cidr_prefix: 24,
    }
}

/// Scripted probe-cycle simulator (MOCK). Owns the scheduler, the queue,
/// the ledger, and the running set. Each tick: finish due probes, sweep
/// heartbeats (crash arm), then ask the scheduler for launches. The
/// scheduler's `in_flight` count and the sim's running set move together.
struct CycleSim {
    sched: Scheduler<ManualClock>,
    queue: TargetQueue,
    ledger: RunLedger,
    /// (run_id, ticks_remaining, last_heartbeat_tick).
    running: Vec<(String, u64, u64)>,
    /// Run ids whose worker crashed: never progress, never heartbeat.
    crashed: HashSet<String>,
    /// Target whose probe worker crashes at launch (A1 only).
    crash_target: Option<TargetId>,
    tick: u64,
    peak: usize,
    next_run: u64,
    launch_ticks: Vec<(TargetId, u64)>,
    failed_at: HashMap<String, u64>,
}

impl CycleSim {
    fn new(
        max_concurrent: u32,
        targets: Vec<Target>,
        crash_target: Option<TargetId>,
    ) -> Result<Self, TaskDriverError> {
        let sched = Scheduler::new(open_program(), max_concurrent, ManualClock::new(0)).map_err(
            |detail| TaskDriverError::Fixture {
                what: "scheduler".to_string(),
                detail: format!("task-136: scheduler construction failed: {detail}"),
            },
        )?;
        let mut queue = TargetQueue::new();
        for t in targets {
            queue.push(t);
        }
        Ok(CycleSim {
            sched,
            queue,
            ledger: RunLedger::new(),
            running: Vec::new(),
            crashed: HashSet::new(),
            crash_target,
            tick: 0,
            peak: 0,
            next_run: 0,
            launch_ticks: Vec::new(),
            failed_at: HashMap::new(),
        })
    }

    fn launch_tick_of(&self, id: &TargetId) -> Option<u64> {
        self.launch_ticks
            .iter()
            .find(|(t, _)| t == id)
            .map(|(_, tick)| *tick)
    }

    /// One simulated tick.
    fn step(&mut self) {
        self.tick += 1;
        // 1. Advance healthy probes; crashed workers never progress.
        let mut finished_ids = Vec::new();
        for entry in self.running.iter_mut() {
            if self.crashed.contains(&entry.0) {
                continue;
            }
            entry.1 = entry.1.saturating_sub(1);
            if entry.1 == 0 {
                finished_ids.push(entry.0.clone());
            }
        }
        for run_id in finished_ids {
            self.ledger.set_state(&run_id, RunState::Finished, None);
            self.sched.note_run_finished();
        }
        self.running
            .retain(|e| e.1 > 0 || self.crashed.contains(&e.0));
        // 2. Heartbeat sweep: a run silent past the timeout is failed and
        //    its scheduler slot released. Healthy workers heartbeat.
        let mut failed_ids = Vec::new();
        for entry in self.running.iter_mut() {
            if self.crashed.contains(&entry.0) {
                if self.tick.saturating_sub(entry.2) > HEARTBEAT_TIMEOUT_TICKS {
                    failed_ids.push(entry.0.clone());
                }
            } else {
                entry.2 = self.tick;
            }
        }
        for run_id in &failed_ids {
            self.ledger.set_state(
                run_id,
                RunState::Failed,
                Some("heartbeat-timeout".to_string()),
            );
            self.sched.note_run_finished();
            self.failed_at.insert(run_id.clone(), self.tick);
        }
        self.running.retain(|e| !failed_ids.contains(&e.0));
        // 3. Launch up to the structural bound.
        let queue = &mut self.queue;
        let mut drain = || queue.pop();
        let actions = self.sched.tick(&mut drain);
        for action in actions {
            if let SchedAction::Launch(target) = action {
                let run_id = format!("r{:03}", self.next_run);
                self.next_run += 1;
                self.ledger.record(Run {
                    id: run_id.clone(),
                    target_id: target.id.clone(),
                    state: RunState::Running,
                    approval_nonce: 0,
                    cancel_reason: None,
                });
                if self.crash_target.as_ref() == Some(&target.id) {
                    self.crashed.insert(run_id.clone());
                }
                self.running.push((run_id, PROBE_TICKS, self.tick));
                self.launch_ticks.push((target.id.clone(), self.tick));
            }
        }
        self.peak = self.peak.max(self.running.len());
    }

    /// Run until the queue and the running set are empty.
    fn drain(&mut self) -> Result<(), TaskDriverError> {
        while (!self.queue.is_empty() || !self.running.is_empty()) && self.tick < TICK_CAP {
            self.step();
        }
        if !self.queue.is_empty() || !self.running.is_empty() {
            return Err(TaskDriverError::Arm {
                arm: "drain".to_string(),
                detail: format!("task-136: sim did not drain within {TICK_CAP} ticks"),
            });
        }
        Ok(())
    }
}

/// V1: 10 targets, K=3, 2-tick probes. Peak concurrent must be exactly
/// 3 and all 10 must finish.
fn case_peak_bound_k3() -> Result<CaseReport, TaskDriverError> {
    let targets: Vec<Target> = (0..TARGET_COUNT).map(mk_target).collect();
    let mut sim = CycleSim::new(3, targets, None)?;
    sim.drain()?;
    let finished = sim.ledger.count_in_state(RunState::Finished);
    let failed = sim.ledger.count_in_state(RunState::Failed);
    let mut evidence = vec![
        format!("K=3, {TARGET_COUNT} targets, probe {PROBE_TICKS} ticks"),
        format!("peak concurrent = {} (bound: exactly 3)", sim.peak),
        format!("finished = {finished}, failed = {failed}"),
        format!("ticks to drain = {}", sim.tick),
        "backend: scripted-mock".to_string(),
    ];
    let mut failures = Vec::new();
    if sim.peak != 3 {
        failures.push(format!("peak {} != 3: bound violated", sim.peak));
    }
    if finished != TARGET_COUNT {
        failures.push(format!("finished {finished} != {TARGET_COUNT}"));
    }
    if failed != 0 {
        failures.push(format!("{failed} unexpected failures"));
    }
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "k": 3,
            "targets": TARGET_COUNT,
            "peak": sim.peak,
            "finished": finished,
            "failed": failed,
            "ticks": sim.tick,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: K=1 is strictly serial — peak 1, every target still completes.
fn case_serial_at_k1() -> Result<CaseReport, TaskDriverError> {
    let targets: Vec<Target> = (0..6).map(mk_target).collect();
    let mut sim = CycleSim::new(1, targets, None)?;
    sim.drain()?;
    let finished = sim.ledger.count_in_state(RunState::Finished);
    let evidence = vec![
        "K=1: strictly serial".to_string(),
        format!("peak concurrent = {} (bound: exactly 1)", sim.peak),
        format!("finished = {finished} / 6"),
        "backend: scripted-mock".to_string(),
    ];
    let mut failures = Vec::new();
    if sim.peak != 1 {
        failures.push(format!("peak {} != 1: serialization broken", sim.peak));
    }
    if finished != 6 {
        failures.push(format!("finished {finished} != 6"));
    }
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "k": 1,
            "targets": 6,
            "peak": sim.peak,
            "finished": finished,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1 (adversarial): the t00 worker crashes at launch — it never
/// progresses and never heartbeats. The watchdog must fail the run,
/// release the scheduler slot within the heartbeat bound, and the
/// queue must keep draining.
fn case_crash_releases_slot() -> Result<CaseReport, TaskDriverError> {
    let targets: Vec<Target> = (0..4).map(mk_target).collect();
    let crash_id = TargetId("t00".to_string());
    let mut sim = CycleSim::new(2, targets, Some(crash_id.clone()))?;
    sim.drain()?;
    let finished = sim.ledger.count_in_state(RunState::Finished);
    let failed = sim.ledger.count_in_state(RunState::Failed);
    let crashed_run = sim
        .ledger
        .runs()
        .iter()
        .find(|r| r.target_id == crash_id)
        .map(|r| (r.id.clone(), r.state.clone(), r.cancel_reason.clone()));
    let failed_tick = crashed_run
        .as_ref()
        .and_then(|(id, _, _)| sim.failed_at.get(id).copied());
    let crash_launch = sim.launch_tick_of(&crash_id).unwrap_or(0);
    let last_launch = sim
        .launch_tick_of(&TargetId("t03".to_string()))
        .unwrap_or(0);
    let mut evidence = vec![
        format!("K=2, 4 targets, t00 worker crashes at launch (tick {crash_launch})"),
        format!("crashed run: {crashed_run:?}"),
        format!(
            "failed_at tick = {failed_tick:?} (bound: launch + {} + 1)",
            HEARTBEAT_TIMEOUT_TICKS
        ),
        format!("t03 (last) launched at tick {last_launch}"),
        format!(
            "peak = {} (bound: <= 2), finished = {finished}, failed = {failed}",
            sim.peak
        ),
        "backend: scripted-mock".to_string(),
    ];
    let mut failures = Vec::new();
    match (&crashed_run, failed_tick) {
        (Some((_, RunState::Failed, Some(reason))), Some(ft))
            if reason == "heartbeat-timeout"
                && ft - crash_launch == HEARTBEAT_TIMEOUT_TICKS + 1 => {}
        _ => failures.push(format!(
            "crashed run not failed by watchdog within bound: {crashed_run:?} at {failed_tick:?}"
        )),
    }
    if finished != 3 {
        failures.push(format!(
            "finished {finished} != 3: queue did not drain past the crash"
        ));
    }
    if failed != 1 {
        failures.push(format!("failed {failed} != 1"));
    }
    if last_launch != failed_tick.unwrap_or(u64::MAX) {
        failures.push(format!(
            "t03 launched at {last_launch}, not at the release tick {failed_tick:?}: \
             slot release did not unblock the queue"
        ));
    }
    if sim.peak > 2 {
        failures.push(format!("peak {} > 2: bound violated", sim.peak));
    }
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "k": 2,
            "targets": 4,
            "peak": sim.peak,
            "finished": finished,
            "failed": failed,
            "failed_at_tick": failed_tick,
            "crash_launch_tick": crash_launch,
            "heartbeat_timeout_ticks": HEARTBEAT_TIMEOUT_TICKS,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 (adversarial): K=0 is refused at construction — fail closed. Zero
/// is never read as "unlimited".
fn case_k0_refused() -> Result<CaseReport, TaskDriverError> {
    match Scheduler::new(open_program(), 0, ManualClock::new(0)) {
        Err(detail) if detail.contains("max_concurrent") => {
            let report = CaseReport::pass(
                CASES[3],
                serde_json::json!({
                    "refused": true,
                    "error": detail,
                    "backend": "scripted-mock",
                }),
                vec![
                    "Scheduler::new(k=0) refused at construction".to_string(),
                    format!("typed refusal: {detail}"),
                    "zero is fail-closed, never unlimited".to_string(),
                    "backend: scripted-mock".to_string(),
                ],
            );
            Ok(report)
        }
        Err(detail) => Ok(CaseReport::fail(
            CASES[3],
            format!("k=0 refused with unexpected error: {detail}"),
            vec!["backend: scripted-mock".to_string()],
        )),
        Ok(_) => Ok(CaseReport::fail(
            CASES[3],
            "Scheduler::new(k=0) SUCCEEDED — zero concurrency accepted".to_string(),
            vec!["backend: scripted-mock".to_string()],
        )),
    }
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "peak_bound_k3" => case_peak_bound_k3(),
        "serial_at_k1" => case_serial_at_k1(),
        "crash_releases_slot" => case_crash_releases_slot(),
        "k0_refused" => case_k0_refused(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-136: unknown case '{case}'"),
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
            where_: "task-136".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-136".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
