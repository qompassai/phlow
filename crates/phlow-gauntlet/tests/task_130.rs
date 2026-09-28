//! Integration tests for task-130 (idle-loop quietness — the
//! phone-battery acceptance test; diver Phase-2 Decision 3).
//!
//! The design justification for "no poll" over 250ms/1s: a polling
//! supervisor wakes the event loop forever; an event-driven one sleeps.
//!
//! The loop is quiet today by absence (no supervision without an explicit
//! `tick()`). The validation half pins the gap records (the Phase-2
//! acceptance artifact: one pending handle per live deadline, a teardown
//! that closes every tracked handle); the adversarial half runs the
//! actual quietness acceptance — a 10s idle window with zero wakeups and
//! 100 settled runs back at the handle baseline. 50/50 split: 2
//! validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_130;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-130: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-130: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-130: HOME is not set"))
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
        "gauntlet-task-130-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-130: cannot build Ctx: {e}"));
    // The idle window itself is 10s; give the driver headroom.
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the scenario to pass; panic on any failure so a regression in
/// the characterization is loud.
fn expect_pass(scenario: &str) -> Vec<String> {
    let ctx = ctx_for(scenario);
    match task_130::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-130 scenario '{scenario}' failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}

/// Expect the quietness gap record; panic on any other outcome (a pass or
/// a driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str, want_where: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_130::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, want_where,
                "task-130 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-130 scenario '{scenario}' passed: the timer surface exists, contradicting the probed gap\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and a 24h-deadline run holds zero
/// pending uv handles — the gap record pins the Phase-2 acceptance
/// (exactly one pending uv handle per live deadline, zero wakeups at
/// idle).
#[test]
fn deadline_handle_gap() {
    assert_eq!(task_130::ID, "task-130");
    assert_eq!(task_130::NAME, "idle-loop-quietness");
    assert_eq!(task_130::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("deadline-handle-absent", "deadline-handle-absent");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("uv timers pending: 0"),
        "evidence must show zero pending handles:\n{joined}"
    );
    assert!(
        how.contains("exactly one pending uv handle"),
        "the 'how' must pin the one-handle acceptance: {how}"
    );
}

/// V: no `VimLeavePre` teardown closes timer handles — the gap record pins
/// the Phase-2 teardown acceptance (teardown closes every tracked
/// handle, emits no errors, handle count returns to baseline).
#[test]
fn timer_teardown_gap() {
    let (how, evidence) = expect_gap("timer-teardown-absent", "timer-teardown-absent");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("'VimLeavePre' matches: 0"),
        "evidence must show the source scan found no teardown:\n{joined}"
    );
    assert!(
        how.contains("closes every tracked handle"),
        "the 'how' must pin the teardown acceptance: {how}"
    );
}

// --- adversarial ---

/// A: the phone-battery test itself — 10s of idle with zero live runs:
/// zero new uv handles, `sup.last_tick_ns` unmoved (no background
/// ticking), zero new sink events. The loop sleeps.
#[test]
fn idle_window_zero_activity_pass() {
    let evidence = expect_pass("idle-window-zero-activity");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("after 10s idle"),
        "evidence must cover the full observation window:\n{joined}"
    );
    assert!(
        joined.contains("last_tick_ns delta=0"),
        "evidence must show no background ticking:\n{joined}"
    );
    assert!(
        joined.contains("sink delta=0"),
        "evidence must show no stray events:\n{joined}"
    );
}

/// A: 100 settled runs leave zero uv handle residue — the handle count
/// returns to the pre-setup baseline.
#[test]
fn settled_runs_baseline_handles_pass() {
    let evidence = expect_pass("settled-runs-baseline-handles");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("100 runs created and settled"),
        "evidence must show all 100 runs settled:\n{joined}"
    );
    assert!(
        joined.contains("zero uv handle residue"),
        "evidence must show the baseline held:\n{joined}"
    );
}
