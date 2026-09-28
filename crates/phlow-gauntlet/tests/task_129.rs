//! Integration tests for task-129 (approval-expiry one-shots; diver
//! Phase-2 Decision 3).
//!
//! The design: an approval request schedules a one-shot at its expiry →
//! `wake(sup, now, 'approval')` → `approval.sweep_expired`; grant/deny
//! cancels the timer.
//!
//! No one-shot exists today — expiry is a timestamp swept by `tick()`.
//! The validation half pins the gap record (the Phase-2 acceptance
//! artifact, with the exact hook location) and the timer-free request
//! path; the adversarial half stresses expiry-with-no-decision (the run
//! is not handled) and the grant/deny-at-T-eps races. 50/50 split: 2
//! validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_129;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-129: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-129: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-129: HOME is not set"))
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
        "gauntlet-task-129-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-129: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the scenario to pass; panic on any failure so a regression in
/// the characterization is loud.
fn expect_pass(scenario: &str) -> Vec<String> {
    let ctx = ctx_for(scenario);
    match task_129::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-129 scenario '{scenario}' failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}

/// Expect the approval-timer gap record; panic on any other outcome (a
/// pass or a driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str, want_where: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_129::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, want_where,
                "task-129 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-129 scenario '{scenario}' passed: the approval one-shot exists, contradicting the probed gap\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and no one-shot fires at approval
/// expiry — the gap record pins the Phase-2 acceptance (one-shot at
/// expiry → `wake(sup, now, 'approval')` → `sweep_expired`; grant/deny
/// cancels the timer; no double-decision; per-run independence) and
/// records the exact hook location (`approval.request`).
#[test]
fn approval_one_shot_gap() {
    assert_eq!(task_129::ID, "task-129");
    assert_eq!(task_129::NAME, "approval-expiry-one-shots");
    assert_eq!(task_129::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("approval-one-shot-absent", "approval-one-shot-absent");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("hook point") && joined.contains("M.request"),
        "evidence must record the exact hook location:\n{joined}"
    );
    assert!(
        how.contains("one-shot at approval expiry") && how.contains("no double-decision"),
        "the 'how' must pin the one-shot acceptance: {how}"
    );
}

/// V: requesting an approval schedules no uv timer and the approval entry
/// carries no timer handle — the request path is timer-free.
#[test]
fn no_approval_timer_pass() {
    let evidence = expect_pass("no-approval-timer");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no timer handle"),
        "evidence must show the entry carries no handle:\n{joined}"
    );
    assert!(
        joined.contains("delta across request is zero"),
        "evidence must show the zero timer delta:\n{joined}"
    );
}

// --- adversarial ---

/// A: expiry with no decision — the sweep marks the approval 'expired'
/// but the run is NOT handled per policy (no transition, no outcome
/// event). The design expects handling; today it silently does not
/// happen, and this test documents that.
#[test]
fn approval_expiry_leaves_run_untouched() {
    let evidence = expect_pass("approval-expiry-via-tick");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("nothing fired at expiry"),
        "evidence must show no auto-sweep:\n{joined}"
    );
    assert!(
        joined.contains("the run itself is untouched"),
        "evidence must show the run is not handled:\n{joined}"
    );
}

/// A: grant at T-eps and deny-then-expiry — the races resolve cleanly
/// under tick serialization: no double-decision, no phantom expiry, and
/// decide/sweep stay silent (zero sink events).
#[test]
fn grant_before_expiry_no_double_decision_pass() {
    let evidence = expect_pass("grant-before-expiry-no-double-decision");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no double-decision"),
        "evidence must show the grant survived the sweep:\n{joined}"
    );
    assert!(
        joined.contains("zero sink events"),
        "evidence must show decide/sweep are silent:\n{joined}"
    );
}
