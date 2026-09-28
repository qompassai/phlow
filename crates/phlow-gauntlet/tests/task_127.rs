//! Integration tests for task-127 (deadline one-shots and handle
//! hygiene; diver Phase-2 Decision 3).
//!
//! The design: run create schedules a one-shot `vim.uv` timer at
//! `deadline_ns` → `wake(sup, now, 'deadline')`; the handle is tracked in
//! `run._timers` and cancelled on terminal entry inside `transition()`
//! (the airtight choke point).
//!
//! No one-shot exists today — the deadline is a timestamp compared inside
//! `tick()`. The two validation tests pin the gap records (the Phase-2
//! acceptance artifact); the two adversarial tests characterize today
//! (tick-only deadline enforcement, no timer at create). 50/50 split: 2
//! validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_127;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-127: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-127: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-127: HOME is not set"))
}

/// Process-local sequence so concurrent `ctx_for` calls never collide.
static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build a `Ctx` for one scenario with its own scratch directory.
///
/// The workdir is unique per call (pid + a process-local counter): two
/// tests running the *same* scenario in parallel get disjoint directories,
/// while the driver still sees the unchanged scenario name.
fn ctx_for(scenario: &str) -> Ctx {
    let nvim_bin = required_dir(
        "GAUNTLET_NVIM_BIN",
        &format!("{}/workspace/tools/neovim-nightly/bin/nvim", home_dir()),
    );
    let diver_lua = required_dir(
        "GAUNTLET_DIVER_LUA",
        &format!("{}/workspace/repos/diver/lua", home_dir()),
    );
    let seq = WORKDIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let work_dir = std::env::temp_dir().join(format!(
        "gauntlet-task-127-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-127: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the scenario to pass; panic on any failure so a regression in
/// the characterization is loud.
fn expect_pass(scenario: &str) -> Vec<String> {
    let ctx = ctx_for(scenario);
    match task_127::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-127 scenario '{scenario}' failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}

/// Expect the deadline-timer gap record; panic on any other outcome (a
/// pass or a driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str, want_where: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_127::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, want_where,
                "task-127 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-127 scenario '{scenario}' passed: the deadline timer exists, contradicting the probed gap\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and no one-shot fires at
/// `deadline_ns` — the gap record pins the Phase-2 acceptance (one-shot
/// at `deadline_ns` → `wake(sup, now, 'deadline')`, handle in
/// `run._timers`, cancelled in `transition()`).
#[test]
fn deadline_one_shot_gap() {
    assert_eq!(task_127::ID, "task-127");
    assert_eq!(task_127::NAME, "deadline-one-shots-handle-hygiene");
    assert_eq!(task_127::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("deadline-one-shot-absent", "deadline-one-shot-absent");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("M.create") && joined.contains("no vim.uv timer"),
        "evidence must show create schedules nothing:\n{joined}"
    );
    assert!(
        how.contains("one-shot at deadline_ns"),
        "the 'how' must pin the one-shot acceptance: {how}"
    );
}

/// V: `transition()` cancels nothing on terminal entry — the gap record
/// pins the handle-hygiene acceptance (handles tracked in `run._timers`,
/// cancelled in `transition()`, zero live handles after terminal).
#[test]
fn timer_handle_hygiene_gap() {
    let (how, evidence) = expect_gap(
        "terminal-entry-no-timer-cleanup",
        "timer-handle-hygiene-absent",
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("transition()") && joined.contains("no _timers cancellation"),
        "evidence must show the choke point has nothing to cancel:\n{joined}"
    );
    assert!(
        how.contains("run._timers") && how.contains("handle-leak assertion"),
        "the 'how' must pin the hygiene acceptance: {how}"
    );
}

// --- adversarial ---

/// A: the deadline is a timestamp compared inside `tick()` — a run past
/// its deadline stays live until an explicit `tick()` times it out with
/// reason 'deadline exceeded'.
#[test]
fn deadline_fires_via_tick_pass() {
    let evidence = expect_pass("deadline-fires-via-tick");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no timer fired"),
        "evidence must show nothing fired at the deadline:\n{joined}"
    );
    assert!(
        joined.contains("deadline exceeded"),
        "evidence must show the tick-timeout reason:\n{joined}"
    );
}

/// A: creating a run schedules no uv timer and tracks no `run._timers` —
/// the timer delta across create is zero.
#[test]
fn no_one_shot_at_create_pass() {
    let evidence = expect_pass("no-one-shot-at-create");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("run._timers == nil"),
        "evidence must show no per-run timer table:\n{joined}"
    );
    assert!(
        joined.contains("delta across create is zero"),
        "evidence must show the zero timer delta:\n{joined}"
    );
}
