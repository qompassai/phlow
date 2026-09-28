//! Integration tests for task-126 (sink-append wakes supervision, no
//! poll; diver Phase-2 Decision 3).
//!
//! The design: the sink gains an `on_append` subscriber hook; the
//! supervisor registers its wake callback at setup; a `waking` reentrancy
//! flag coalesces storms; no periodic timer exists. `tick()` is retained
//! as the test driver.
//!
//! The wake surface does not exist today: the two validation tests pin
//! the gap records (the Phase-2 acceptance artifact), and the two
//! adversarial tests characterize today — zero repeating timers after
//! setup (the no-poll regression guard) and tick() as the sole
//! supervision driver. 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_126;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-126: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-126: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-126: HOME is not set"))
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
        "gauntlet-task-126-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-126: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the scenario to pass; panic on any failure so a regression in
/// the characterization is loud.
fn expect_pass(scenario: &str) -> Vec<String> {
    let ctx = ctx_for(scenario);
    match task_126::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-126 scenario '{scenario}' failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}

/// Expect the wake-surface gap record; panic on any other outcome (a pass
/// or a driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str, want_where: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_126::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, want_where,
                "task-126 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-126 scenario '{scenario}' passed: the wake surface exists, contradicting the probed gap\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and the sink has no `on_append`
/// hook — the gap record pins the Phase-2 acceptance (subscriber hook in
/// pcall on every append, wake registered at setup, wake runs the tick
/// body, `waking` reentrancy flag).
#[test]
fn wake_hook_gap() {
    assert_eq!(task_126::ID, "task-126");
    assert_eq!(task_126::NAME, "sink-append-wake-no-poll");
    assert_eq!(task_126::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("wake-hook-absent", "sink-wake-hook-absent");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("sink.on_append == nil"),
        "evidence must show the hook is absent:\n{joined}"
    );
    assert!(
        joined.contains("supervisor.wake == nil"),
        "evidence must show no wake callback exists:\n{joined}"
    );
    assert!(
        how.contains("on_append") && how.contains("waking"),
        "the 'how' must pin the Phase-2 wake contract: {how}"
    );
}

/// V: no `waking` coalescing flag exists — the storm gap record pins the
/// bounded-wake acceptance (<= 3 wake passes per 1000 appends, no
/// recursion on nested append, exactly one finish per run).
#[test]
fn wake_coalescing_gap() {
    let (how, evidence) = expect_gap("wake-coalescing-absent", "wake-coalescing-absent");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("1000 rapid appends"),
        "evidence must show exactly 1000 appends in the storm:\n{joined}"
    );
    assert!(
        joined.contains("no wake fired"),
        "evidence must show the storm caused zero supervision passes:\n{joined}"
    );
    assert!(
        joined.contains("exactly one run.finished each"),
        "evidence must show the drain itself is storm-safe:\n{joined}"
    );
    assert!(
        how.contains("<= 3 wake passes"),
        "the 'how' must pin the coalescing bound: {how}"
    );
}

// --- adversarial ---

/// A: the no-poll regression guard — setup leaves zero repeating uv
/// timers. Passes trivially today; a future periodic timer fails it by
/// construction.
#[test]
fn no_repeating_timers_pass() {
    let evidence = expect_pass("no-repeating-timers");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("repeating=0"),
        "evidence must show zero repeating timers:\n{joined}"
    );
    assert!(
        joined.contains("no polling loop"),
        "evidence must name the regression guard:\n{joined}"
    );
}

/// A: a `model.completed` appended straight to the sink leaves the run
/// live until an explicit `tick()` drains it — tick() is the sole
/// supervision driver today.
#[test]
fn tick_drives_completions_pass() {
    let evidence = expect_pass("tick-drives-completions");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("still running"),
        "evidence must show no auto-wake on append:\n{joined}"
    );
    assert!(
        joined.contains("explicit tick()"),
        "evidence must show the drain ran on tick:\n{joined}"
    );
    assert!(
        joined.contains("drain_completions"),
        "evidence must cite the drain source:\n{joined}"
    );
}
