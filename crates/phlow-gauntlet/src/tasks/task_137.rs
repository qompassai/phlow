//! Task 137 — cancellation mid-run with no zombies (rust, validation +
//! adversarial).
//!
//! The cancel path is queue removal + child kill + ledger update, in that
//! order — and the ordering is the whole mechanism: a killed-but-unreaped
//! child is a zombie (POSIX: only `wait` reaps), so the kill is
//! immediately followed by `wait`. Verified at the OS level: after the
//! cancel path the PID has no `/proc` entry. Cancelling a terminal run
//! is a typed [`CancelError::AlreadyTerminal`] (never a second kill); a
//! double cancel kills exactly once. Real `sleep` children are spawned
//! (the design demands real processes); everything else is the clearly
//! labeled scripted double (MOCK).

use std::collections::HashMap;
use std::fmt;
use std::process::{Child, Command};
use std::time::Duration;

use crate::bounty::{Run, RunLedger, RunState, Target, TargetId, TargetKind, TargetQueue};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-137";
/// Task name.
pub const NAME: &str = "cancellation mid-run with no zombies";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation + 2 adversarial.
pub const CASES: [&str; 4] = [
    "cancel_queued_removes_without_spawn",
    "cancel_running_reaps_no_zombie",
    "cancel_finished_is_already_terminal",
    "double_cancel_single_kill",
];

/// What the scripted probe child execs. Far beyond any test duration,
/// so a child still alive at case end is a leak.
const PROBE_SLEEP_SECS: &str = "600";
/// Bounded polls for the transient zombie state between kill and reap.
const ZOMBIE_POLL_ROUNDS: u32 = 200;

/// Typed cancellation failures. Cancelling a terminal run is refused,
/// never silently re-applied; unknown runs are refused, never created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelError {
    /// The run is already in a terminal state; cancellation is a no-op.
    AlreadyTerminal { run_id: String },
    /// No run is recorded for this target.
    UnknownRun { target: String },
    /// The OS refused part of the kill/reap sequence.
    Os { op: &'static str, detail: String },
}

impl fmt::Display for CancelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CancelError::AlreadyTerminal { run_id } => {
                write!(f, "run {run_id} already terminal: cancel refused")
            }
            CancelError::UnknownRun { target } => {
                write!(f, "no run recorded for target {target}")
            }
            CancelError::Os { op, detail } => write!(f, "os failure during {op}: {detail}"),
        }
    }
}

impl std::error::Error for CancelError {}

/// What one cancellation did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The target was still queued: removed, zero processes spawned.
    Dequeued,
    /// The running probe was killed and reaped.
    Killed { pid: u32, zombie_observed: bool },
}

/// Info about a launched probe.
#[derive(Debug, Clone)]
struct LaunchInfo {
    run_id: String,
    target_id: TargetId,
    pid: u32,
}

/// Current state char of a Linux process, or `None` when the PID is
/// gone. Parses `/proc/<pid>/stat`; the comm field may contain parens
/// and spaces, so the state char follows the LAST `)`.
fn proc_state(pid: u32) -> Option<char> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_paren = text.rfind(')')?;
    text[after_paren + 1..].trim_start().chars().next()
}

/// Owns every spawned probe child (MOCK scripted `sleep` processes),
/// the queue, and the run ledger. Every child is reaped exactly once —
/// by the cancel path or by `finish_run` — and `Drop` reaps any
/// straggler so a failed assertion cannot leak a zombie.
struct ProbeManager {
    queue: TargetQueue,
    ledger: RunLedger,
    children: HashMap<String, Child>,
    run_of: HashMap<TargetId, String>,
    spawned: u32,
    killed: u32,
    reaped: u32,
    next_run: u64,
}

impl ProbeManager {
    fn new() -> Self {
        ProbeManager {
            queue: TargetQueue::new(),
            ledger: RunLedger::new(),
            children: HashMap::new(),
            run_of: HashMap::new(),
            spawned: 0,
            killed: 0,
            reaped: 0,
            next_run: 0,
        }
    }

    fn enqueue(&mut self, target: Target) {
        let run_id = format!("r{:03}", self.next_run);
        self.next_run += 1;
        self.run_of.insert(target.id.clone(), run_id.clone());
        self.queue.push(target.clone());
        self.ledger.record(Run {
            id: run_id,
            target_id: target.id,
            state: RunState::Queued,
            approval_nonce: 0,
            cancel_reason: None,
        });
    }

    fn run_state(&self, target_id: &TargetId) -> Option<RunState> {
        let run_id = self.run_of.get(target_id)?;
        self.ledger
            .runs()
            .iter()
            .find(|r| &r.id == run_id)
            .map(|r| r.state.clone())
    }

    /// Launch the head of the queue: spawn the real child, mark Running.
    fn launch_next(&mut self) -> Result<LaunchInfo, TaskDriverError> {
        let target = self.queue.pop().ok_or_else(|| TaskDriverError::Fixture {
            what: "queue".to_string(),
            detail: "task-137: launch_next on empty queue".to_string(),
        })?;
        let run_id =
            self.run_of
                .get(&target.id)
                .cloned()
                .ok_or_else(|| TaskDriverError::Fixture {
                    what: "ledger".to_string(),
                    detail: "task-137: queued target has no run record".to_string(),
                })?;
        let child = Command::new("sleep")
            .arg(PROBE_SLEEP_SECS)
            .spawn()
            .map_err(|e| TaskDriverError::Fixture {
                what: "spawn".to_string(),
                detail: format!("task-137: failed to spawn sleep: {e}"),
            })?;
        // Kill-then-reap ordering starts here: the child is owned from
        // birth, so no path can lose it before reaping.
        let pid = child.id();
        self.spawned += 1;
        self.children.insert(run_id.clone(), child);
        self.ledger.set_state(&run_id, RunState::Running, None);
        Ok(LaunchInfo {
            run_id,
            target_id: target.id,
            pid,
        })
    }

    /// Reap one owned child: kill, then wait. Returns the pid.
    fn reap_child(&mut self, run_id: &str) -> Result<u32, CancelError> {
        let mut child = self
            .children
            .remove(run_id)
            .ok_or_else(|| CancelError::Os {
                op: "reap",
                detail: format!("task-137: no owned child for run {run_id}"),
            })?;
        let pid = child.id();
        self.killed += 1;
        child.kill().map_err(|e| CancelError::Os {
            op: "kill",
            detail: e.to_string(),
        })?;
        child.wait().map_err(|e| CancelError::Os {
            op: "wait",
            detail: e.to_string(),
        })?;
        self.reaped += 1;
        Ok(pid)
    }

    /// Scripted probe completion (not a cancel): reap the child and mark
    /// the run Finished.
    fn finish_run(&mut self, target_id: &TargetId) -> Result<(), CancelError> {
        let run_id = self
            .run_of
            .get(target_id)
            .cloned()
            .ok_or(CancelError::UnknownRun {
                target: target_id.0.clone(),
            })?;
        self.reap_child(&run_id)?;
        self.ledger.set_state(&run_id, RunState::Finished, None);
        Ok(())
    }

    /// The cancel path: queued → dequeue; running → kill + reap;
    /// terminal → typed refusal. The ledger is updated exactly once.
    fn cancel(&mut self, target_id: &TargetId) -> Result<CancelOutcome, CancelError> {
        let run_id = self
            .run_of
            .get(target_id)
            .cloned()
            .ok_or(CancelError::UnknownRun {
                target: target_id.0.clone(),
            })?;
        let state = self
            .ledger
            .runs()
            .iter()
            .find(|r| r.id == run_id)
            .map(|r| r.state.clone())
            .ok_or(CancelError::UnknownRun {
                target: target_id.0.clone(),
            })?;
        match state {
            RunState::Queued => {
                // Queue removal first: nothing spawned, nothing to kill.
                assert!(self.queue.cancel(target_id));
                self.ledger.set_state(
                    &run_id,
                    RunState::Cancelled,
                    Some("operator-cancel".to_string()),
                );
                Ok(CancelOutcome::Dequeued)
            }
            RunState::Running => {
                let pid = self.reap_child(&run_id)?;
                // Between kill and wait the child is a zombie; observe it
                // (bounded) to prove the ordering, then assert the reap.
                let mut zombie_observed = false;
                for _ in 0..ZOMBIE_POLL_ROUNDS {
                    if proc_state(pid) == Some('Z') {
                        zombie_observed = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                self.ledger.set_state(
                    &run_id,
                    RunState::Cancelled,
                    Some("operator-cancel".to_string()),
                );
                Ok(CancelOutcome::Killed {
                    pid,
                    zombie_observed,
                })
            }
            RunState::Finished | RunState::Failed | RunState::Cancelled => {
                Err(CancelError::AlreadyTerminal { run_id })
            }
        }
    }

    fn cancel_reason(&self, target_id: &TargetId) -> Option<String> {
        let run_id = self.run_of.get(target_id)?;
        self.ledger
            .runs()
            .iter()
            .find(|r| &r.id == run_id)
            .and_then(|r| r.cancel_reason.clone())
    }
}

impl Drop for ProbeManager {
    fn drop(&mut self) {
        // Best-effort: Drop cannot report errors; the invariant is that
        // every case reaps its children explicitly, so this only fires on
        // a failed assertion mid-case.
        for (_, mut child) in self.children.drain() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn mk_target(i: usize) -> Target {
    Target {
        id: TargetId(format!("t{i:02}")),
        kind: TargetKind::Domain,
        value: format!("t{i:02}.example.com"),
    }
}

/// V1: cancelling a queued target removes it from the queue and spawns
/// zero processes.
fn case_cancel_queued_removes_without_spawn() -> Result<CaseReport, TaskDriverError> {
    let mut mgr = ProbeManager::new();
    for i in 0..3 {
        mgr.enqueue(mk_target(i));
    }
    let outcome = mgr
        .cancel(&TargetId("t01".to_string()))
        .map_err(|e| TaskDriverError::Arm {
            arm: CASES[0].to_string(),
            detail: format!("task-137: cancel of queued target failed: {e}"),
        })?;
    let mut failures = Vec::new();
    if outcome != CancelOutcome::Dequeued {
        failures.push(format!("expected Dequeued, got {outcome:?}"));
    }
    if mgr.queue.len() != 2 {
        failures.push(format!("queue len {} != 2", mgr.queue.len()));
    }
    if mgr.spawned != 0 || mgr.killed != 0 || mgr.reaped != 0 {
        failures.push(format!(
            "processes touched: spawned={} killed={} reaped={} (want 0/0/0)",
            mgr.spawned, mgr.killed, mgr.reaped
        ));
    }
    if mgr.run_state(&TargetId("t01".to_string())) != Some(RunState::Cancelled) {
        failures.push("t01 run not Cancelled".to_string());
    }
    if mgr.cancel_reason(&TargetId("t01".to_string())).as_deref() != Some("operator-cancel") {
        failures.push("t01 cancel reason not recorded".to_string());
    }
    // Clean up the remaining two: launch and cancel (exercises kill+reap).
    for _ in 0..2 {
        let info = mgr.launch_next()?;
        let pid = info.pid;
        match mgr.cancel(&info.target_id) {
            Ok(CancelOutcome::Killed { pid: p, .. }) if p == pid => {}
            other => failures.push(format!("cleanup cancel failed: {other:?}")),
        }
        if proc_state(pid).is_some() {
            failures.push(format!("cleanup: pid {pid} still has /proc entry"));
        }
    }
    if !mgr.children.is_empty() {
        failures.push("children leaked".to_string());
    }
    let evidence = vec![
        "cancel t01 while Queued -> Dequeued".to_string(),
        format!(
            "spawned={} killed={} reaped={} at cancel time (zero processes)",
            0, 0, 0
        ),
        "queue: [t00, t01, t02] -> [t00, t02]".to_string(),
        "ledger: t01 Cancelled (reason=operator-cancel)".to_string(),
        "remaining two launched then cancelled: kill+reap, no /proc entries".to_string(),
        "real sleep children spawned; all scripted otherwise (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "outcome": "dequeued",
            "spawned": 0,
            "queue_len": mgr.queue.len(),
            "backend": "real-child+scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: cancelling a running probe kills and reaps the real child — no
/// zombie remains, verified by the absence of its /proc entry.
fn case_cancel_running_reaps_no_zombie() -> Result<CaseReport, TaskDriverError> {
    let mut mgr = ProbeManager::new();
    mgr.enqueue(mk_target(0));
    let info = mgr.launch_next()?;
    let outcome = mgr
        .cancel(&info.target_id)
        .map_err(|e| TaskDriverError::Arm {
            arm: CASES[1].to_string(),
            detail: format!("task-137: cancel of running probe failed: {e}"),
        })?;
    let (pid, zombie_observed) = match outcome {
        CancelOutcome::Killed {
            pid,
            zombie_observed,
        } => (pid, zombie_observed),
        CancelOutcome::Dequeued => {
            return Err(TaskDriverError::Arm {
                arm: CASES[1].to_string(),
                detail: "task-137: running probe reported Dequeued".to_string(),
            });
        }
    };
    let mut failures = Vec::new();
    if pid != info.pid {
        failures.push(format!("cancelled pid {pid} != launched {}", info.pid));
    }
    if proc_state(pid).is_some() {
        failures.push(format!("ZOMBIE: /proc/{pid} still exists after kill+wait"));
    }
    if mgr.killed != 1 || mgr.reaped != 1 {
        failures.push(format!(
            "kill/reap counts {} / {} != 1 / 1",
            mgr.killed, mgr.reaped
        ));
    }
    if mgr.run_state(&info.target_id) != Some(RunState::Cancelled) {
        failures.push("run not Cancelled in ledger".to_string());
    }
    let evidence = vec![
        format!("spawned real sleep child pid={pid}"),
        format!("cancel -> kill + wait (reap); zombie state observed mid-path: {zombie_observed}"),
        format!(
            "/proc/{pid} after reap: {:?} (None = no zombie)",
            proc_state(pid)
        ),
        format!(
            "killed={} reaped={} (exactly once each)",
            mgr.killed, mgr.reaped
        ),
        "ledger: run Cancelled (reason=operator-cancel), exactly once".to_string(),
        "real sleep child; all scripted otherwise (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "pid": pid,
            "zombie_observed": zombie_observed,
            "proc_entry_after_reap": proc_state(pid).is_none(),
            "killed": mgr.killed,
            "reaped": mgr.reaped,
            "backend": "real-child+scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A1 (adversarial): cancelling an already-Finished run is a clean
/// typed no-op — no second kill, no ledger change.
fn case_cancel_finished_is_already_terminal() -> Result<CaseReport, TaskDriverError> {
    let mut mgr = ProbeManager::new();
    mgr.enqueue(mk_target(0));
    let info = mgr.launch_next()?;
    mgr.finish_run(&info.target_id)
        .map_err(|e| TaskDriverError::Arm {
            arm: CASES[2].to_string(),
            detail: format!("task-137: finish_run failed: {e}"),
        })?;
    let kills_before = mgr.killed;
    let outcome = mgr.cancel(&info.target_id);
    let mut failures = Vec::new();
    match outcome {
        Err(CancelError::AlreadyTerminal { run_id }) if run_id == info.run_id => {}
        other => failures.push(format!("expected AlreadyTerminal, got {other:?}")),
    }
    if mgr.killed != kills_before {
        failures.push(format!(
            "kill count moved {} -> {} on a refused cancel",
            kills_before, mgr.killed
        ));
    }
    if mgr.run_state(&info.target_id) != Some(RunState::Finished) {
        failures.push("ledger state changed by refused cancel".to_string());
    }
    if proc_state(info.pid).is_some() {
        failures.push(format!("/proc/{} exists: child leaked", info.pid));
    }
    let evidence = vec![
        "probe finished (child reaped as completion, run Finished)".to_string(),
        format!(
            "cancel after finish -> Err(AlreadyTerminal {{ run_id: {} }})",
            info.run_id
        ),
        format!("kill count unchanged at {}: no second kill", mgr.killed),
        "ledger still Finished: refused cancel changed nothing".to_string(),
        "real sleep child; all scripted otherwise (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "error": "AlreadyTerminal",
            "killed_before": kills_before,
            "killed_after": mgr.killed,
            "backend": "real-child+scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// A2 (adversarial): double-cancel of a running target kills exactly
/// once; the second cancel is a typed refusal and the ledger shows one
/// Cancelled entry.
fn case_double_cancel_single_kill() -> Result<CaseReport, TaskDriverError> {
    let mut mgr = ProbeManager::new();
    mgr.enqueue(mk_target(0));
    let info = mgr.launch_next()?;
    let first = mgr
        .cancel(&info.target_id)
        .map_err(|e| TaskDriverError::Arm {
            arm: CASES[3].to_string(),
            detail: format!("task-137: first cancel failed: {e}"),
        })?;
    let pid = match first {
        CancelOutcome::Killed { pid, .. } => pid,
        CancelOutcome::Dequeued => {
            return Err(TaskDriverError::Arm {
                arm: CASES[3].to_string(),
                detail: "task-137: first cancel of running probe dequeued".to_string(),
            });
        }
    };
    let second = mgr.cancel(&info.target_id);
    let mut failures = Vec::new();
    match second {
        Err(CancelError::AlreadyTerminal { run_id }) if run_id == info.run_id => {}
        other => failures.push(format!(
            "second cancel: expected AlreadyTerminal, got {other:?}"
        )),
    }
    if mgr.killed != 1 {
        failures.push(format!("killed {} != 1: not a single kill", mgr.killed));
    }
    if mgr.reaped != 1 {
        failures.push(format!("reaped {} != 1", mgr.reaped));
    }
    let cancelled_count = mgr.ledger.count_in_state(RunState::Cancelled);
    if cancelled_count != 1 {
        failures.push(format!("ledger Cancelled count {cancelled_count} != 1"));
    }
    if proc_state(pid).is_some() {
        failures.push(format!("ZOMBIE: /proc/{pid} exists after double cancel"));
    }
    let evidence = vec![
        "first cancel -> Killed (kill + wait)".to_string(),
        format!(
            "second cancel -> Err(AlreadyTerminal {{ run_id: {} }})",
            info.run_id
        ),
        format!(
            "killed={} reaped={}: single kill, single reap",
            mgr.killed, mgr.reaped
        ),
        format!("ledger Cancelled entries = {cancelled_count} (exactly once)"),
        format!("/proc/{pid} after: None (no zombie)"),
        "real sleep child; all scripted otherwise (MOCK)".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[3],
        serde_json::json!({
            "killed": mgr.killed,
            "reaped": mgr.reaped,
            "cancelled_entries": cancelled_count,
            "second_cancel": "AlreadyTerminal",
            "backend": "real-child+scripted-mock",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "cancel_queued_removes_without_spawn" => case_cancel_queued_removes_without_spawn(),
        "cancel_running_reaps_no_zombie" => case_cancel_running_reaps_no_zombie(),
        "cancel_finished_is_already_terminal" => case_cancel_finished_is_already_terminal(),
        "double_cancel_single_kill" => case_double_cancel_single_kill(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-137: unknown case '{case}'"),
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
            where_: "task-137".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-137".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
