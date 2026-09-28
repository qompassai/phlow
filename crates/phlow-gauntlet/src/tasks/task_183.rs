// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 183 — cancelled bridge reaping (rust, A).
//!
//! The seam is task cancellation → bridge teardown. Cancelling a task
//! reaps everything: the bridge process, the DevTools target, no
//! orphan targets, no zombies. A1: cancel mid-`Runtime.evaluate`
//! (long-running script) → the in-flight evaluate fails typed
//! (`BridgeError::Cancelled`), the bridge is killed within
//! [`BRIDGE_KILL_TIMEOUT`](crate::bridge::BRIDGE_KILL_TIMEOUT), the
//! target closes, the port shows zero targets. A2: the bridge process
//! ignores SIGTERM → SIGKILL escalation still reaps it; the process
//! census is zombie-free. Own-and-release-exactly-once on every path.

use crate::bridge::{
    BRIDGE_KILL_TIMEOUT, Bridge, BridgeConfig, BridgeError, DevToolsPort, SharedScriptedPort,
    StubKind, process_census, zombie_pids,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Task id.
pub const ID: &str = "task-183";
/// Task name.
pub const NAME: &str = "cancelled bridge reaping";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = ["cancel_mid_evaluate_reaps", "sigterm_ignored_escalates"];

fn config(task: &str, stub: StubKind) -> BridgeConfig {
    BridgeConfig {
        task_id: task.to_string(),
        allowed_hosts: vec!["example.com".to_string()],
        allow_multi_target: false,
        max_targets: 1,
        stub,
    }
}

/// A1: cancel mid-`Runtime.evaluate`. The scripted port runs a slow
/// evaluate (it polls the shared cancel flag); the worker thread sits
/// inside it while the main thread cancels. The in-flight evaluate
/// must fail typed, and teardown must finish within the kill timeout
/// with zero targets left on the port and no zombie.
fn case_cancel_mid_evaluate_reaps() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let shared = SharedScriptedPort::default();
    let handle = shared.handle();
    handle.lock().unwrap().set_slow_evaluate(true);
    let bridge =
        Bridge::launch(config("T-183a", StubKind::Sleep), Box::new(shared)).map_err(fixture_err)?;
    let pid = bridge.pid();
    let cell: Arc<Mutex<Option<Bridge>>> = Arc::new(Mutex::new(Some(bridge)));
    let worker_cell = Arc::clone(&cell);
    let worker = std::thread::spawn(move || {
        let mut taken = worker_cell.lock().unwrap().take().expect("bridge taken");
        let result = taken.evaluate("long-running()");
        worker_cell.lock().unwrap().replace(taken);
        result
    });
    // Let the evaluate get in flight, then cancel from the main
    // thread through the port handle (the same flag the slow loop
    // polls) — genuine mid-evaluate cancellation.
    std::thread::sleep(Duration::from_millis(300));
    handle.lock().unwrap().set_cancel();
    let start = Instant::now();
    let eval_result = worker.join().expect("worker thread panicked");
    match eval_result {
        Err(BridgeError::Cancelled) => {
            evidence.push("in-flight evaluate -> BridgeError::Cancelled (typed)".to_string());
        }
        other => failures.push(format!("evaluate outcome {other:?}, want Err(Cancelled)")),
    }
    let taken = cell.lock().unwrap().take().expect("bridge returned");
    let report = taken.cancel();
    let elapsed = start.elapsed();
    if !report.reaped {
        failures.push(format!("bridge pid {pid} not reaped"));
    }
    if elapsed >= BRIDGE_KILL_TIMEOUT {
        failures.push(format!(
            "teardown took {elapsed:?}, bound is {BRIDGE_KILL_TIMEOUT:?}"
        ));
    }
    let remaining = handle.lock().unwrap().targets().len();
    if remaining != 0 {
        failures.push(format!("{remaining} targets linger on the port"));
    }
    let zombies = zombie_pids();
    if zombies.contains(&pid) {
        failures.push(format!("pid {pid} is a zombie"));
    }
    let gone = !process_census().iter().any(|e| e.pid == pid);
    if !gone {
        failures.push(format!("pid {pid} still in process census"));
    }
    evidence.push(format!(
        "cancel: reaped={}, elapsed={elapsed:?} (< {BRIDGE_KILL_TIMEOUT:?}), targets=0, zombie=false, census_clean={gone}",
        report.reaped
    ));
    finish_case(
        CASES[0],
        failures,
        evidence,
        serde_json::json!({
            "reaped": report.reaped,
            "elapsed_ms": elapsed.as_millis() as u64,
            "targets_remaining": remaining,
            "zombie": false,
        }),
    )
}

/// A2: the bridge child ignores SIGTERM (python3 stub with the signal
/// masked). `cancel` must escalate to SIGKILL and still reap the
/// process — no zombie, inside the timeout.
fn case_sigterm_ignored_escalates() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let shared = SharedScriptedPort::default();
    let handle = shared.handle();
    let bridge = Bridge::launch(config("T-183b", StubKind::IgnoreSigterm), Box::new(shared))
        .map_err(fixture_err)?;
    let pid = bridge.pid();
    if !bridge_is_alive(pid) {
        failures.push("SIGTERM-ignoring stub not alive after launch".to_string());
    }
    let start = Instant::now();
    let report = bridge.cancel();
    let elapsed = start.elapsed();
    if !report.escalated {
        failures.push("SIGKILL escalation did not fire for a SIGTERM-ignoring child".to_string());
    }
    if !report.reaped {
        failures.push(format!("pid {pid} not reaped after SIGKILL"));
    }
    if elapsed >= BRIDGE_KILL_TIMEOUT {
        failures.push(format!(
            "teardown took {elapsed:?}, bound is {BRIDGE_KILL_TIMEOUT:?}"
        ));
    }
    let zombies = zombie_pids();
    if zombies.contains(&pid) {
        failures.push(format!("pid {pid} is a zombie after SIGKILL"));
    }
    let gone = !process_census().iter().any(|e| e.pid == pid);
    if !gone {
        failures.push(format!("pid {pid} still in process census after SIGKILL"));
    }
    let remaining = handle.lock().unwrap().targets().len();
    if remaining != 0 {
        failures.push(format!("{remaining} targets linger on the port"));
    }
    evidence.push(format!(
        "SIGTERM ignored -> escalated={}, reaped={}, elapsed={elapsed:?}, targets=0, zombie=false, census_clean={gone}",
        report.escalated, report.reaped
    ));
    finish_case(
        CASES[1],
        failures,
        evidence,
        serde_json::json!({
            "escalated": report.escalated,
            "reaped": report.reaped,
            "elapsed_ms": elapsed.as_millis() as u64,
            "targets_remaining": remaining,
            "zombie": false,
        }),
    )
}

/// True while `pid` exists in the census and is not a zombie.
fn bridge_is_alive(pid: u32) -> bool {
    process_census()
        .iter()
        .any(|e| e.pid == pid && e.state != 'Z')
}

fn fixture_err(e: BridgeError) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: "bridge".to_string(),
        detail: format!("task-183: {e}"),
    }
}

fn finish_case(
    case: &'static str,
    failures: Vec<String>,
    mut evidence: Vec<String>,
    metrics: serde_json::Value,
) -> Result<CaseReport, TaskDriverError> {
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(case, metrics, evidence);
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "cancel_mid_evaluate_reaps" => case_cancel_mid_evaluate_reaps(),
        "sigterm_ignored_escalates" => case_sigterm_ignored_escalates(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-183: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// mid-evaluate cancel itself.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-183".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-183".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
