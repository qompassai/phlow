//! Integration tests for task-128 (retry one-shot lifecycle; diver
//! Phase-2 Decision 3).
//!
//! The design: `retry_run` schedules a one-shot at `retry_at_ns` →
//! `wake(sup, now, 'retry')` → re-queue and relaunch; cancel during
//! `retry_wait` cancels the timer.
//!
//! No one-shot exists today — the retry is a timestamp promoted by
//! `tick()`. The validation half pins the gap record (the Phase-2
//! acceptance artifact) and characterizes tick-only promotion; the
//! adversarial half pushes the ceiling and the cancel-during-retry_wait
//! race. 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_128;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-128: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-128: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-128: HOME is not set"))
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
        "gauntlet-task-128-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-128: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the scenario to pass; panic on any failure so a regression in
/// the characterization is loud.
fn expect_pass(scenario: &str) -> Vec<String> {
    let ctx = ctx_for(scenario);
    match task_128::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-128 scenario '{scenario}' failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}

/// Expect the retry-timer gap record; panic on any other outcome (a pass
/// or a driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str, want_where: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_128::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, want_where,
                "task-128 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-128 scenario '{scenario}' passed: the retry one-shot exists, contradicting the probed gap\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and no one-shot is scheduled at
/// `retry_at_ns` — the gap record pins the Phase-2 acceptance (one-shot
/// at `retry_at_ns` → `wake(sup, now, 'retry')` → re-queue + relaunch,
/// attempt incremented exactly once).
#[test]
fn retry_one_shot_gap() {
    assert_eq!(task_128::ID, "task-128");
    assert_eq!(task_128::NAME, "retry-one-shot-lifecycle");
    assert_eq!(task_128::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("retry-one-shot-absent", "retry-one-shot-absent");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("M.retry_run") && joined.contains("timestamp only"),
        "evidence must show retry_run sets a timestamp, not a timer:\n{joined}"
    );
    assert!(
        how.contains("one-shot") && how.contains("attempt incremented exactly once"),
        "the 'how' must pin the one-shot acceptance: {how}"
    );
}

/// V: the retry promotes only via an explicit `tick()` at/after
/// `retry_at_ns` — re-queued and relaunched exactly once, attempt
/// incremented exactly once (never early, never twice).
#[test]
fn retry_promotes_via_tick_pass() {
    let evidence = expect_pass("retry-promotes-via-tick");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no early promotion"),
        "evidence must show the pre-due tick did nothing:\n{joined}"
    );
    assert!(
        joined.contains("exactly once"),
        "evidence must show exactly-once relaunch:\n{joined}"
    );
}

// --- adversarial ---

/// A: retry past `RETRY_ATTEMPTS_MAX` refuses with 'retry attempt ceiling
/// exceeded' before any mutation — run untouched, `retry_at_ns`
/// unchanged, no timer scheduled.
#[test]
fn retry_ceiling_refusal_pass() {
    let evidence = expect_pass("retry-ceiling-refusal");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("retry attempt ceiling exceeded"),
        "evidence must show the ceiling refusal:\n{joined}"
    );
    assert!(
        joined.contains("run untouched"),
        "evidence must show no state change:\n{joined}"
    );
    assert!(
        joined.contains("no timer scheduled"),
        "evidence must show no timer was scheduled:\n{joined}"
    );
}

/// A: cancel during `retry_wait`, then tick far past `retry_at_ns` — the
/// stale timestamp never causes a phantom relaunch; the adapter's `start`
/// is never invoked again.
#[test]
fn cancel_during_retry_wait_pass() {
    let evidence = expect_pass("cancel-during-retry-wait");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no phantom relaunch"),
        "evidence must show the stale timestamp is inert:\n{joined}"
    );
    assert!(
        joined.contains("start count still 1"),
        "evidence must show the adapter was not re-invoked:\n{joined}"
    );
}
