//! Integration tests for task-02: budget exhaustion fails closed.
//!
//! 50/50 split: two validation tests (the default scenario's exhausted
//! terminal state and non-success verdict) and two adversarial tests
//! (zero budget rejected at creation; consumption after exhaustion
//! rejected without resurrection).
//!
//! Each test builds a [`Ctx`] from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`
//! with the pinned defaults as fallback, and a per-scenario scratch dir
//! under the system temp dir. Missing paths panic with a clear message:
//! fail closed, never skip.

use phlow_gauntlet::{Ctx, TaskOutcome, tasks::task_02};
use std::env;
use std::path::PathBuf;

/// Pinned default for the headless Neovim binary.
fn default_nvim_bin() -> PathBuf {
    PathBuf::from(env::var("HOME").expect("HOME must be set"))
        .join("workspace/tools/neovim-nightly/bin/nvim")
}

/// Pinned default for diver's `lua/` directory (the real harness source).
fn default_diver_lua_dir() -> PathBuf {
    PathBuf::from(env::var("HOME").expect("HOME must be set")).join("workspace/repos/diver/lua")
}

/// Build the task context for one scenario. Panics on missing paths.
fn ctx_for(scenario: &str) -> Ctx {
    let nvim_bin = env::var("GAUNTLET_NVIM_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_nvim_bin());
    let diver_lua_dir = env::var("GAUNTLET_DIVER_LUA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_diver_lua_dir());
    assert!(
        nvim_bin.is_file(),
        "task-02: missing headless nvim binary at {} (set GAUNTLET_NVIM_BIN)",
        nvim_bin.display()
    );
    assert!(
        diver_lua_dir.is_dir(),
        "task-02: missing diver lua dir at {} (set GAUNTLET_DIVER_LUA)",
        diver_lua_dir.display()
    );
    let work_dir = env::temp_dir().join(format!("gauntlet-task-02-{scenario}"));
    Ctx::new(nvim_bin, diver_lua_dir, work_dir).expect("task-02: Ctx::new rejected valid paths")
}

/// Unwrap a passing outcome; a driver failure becomes a test failure with
/// the where/how and evidence the driver reported.
fn require_pass(outcome: TaskOutcome) -> Vec<String> {
    match outcome {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-02 driver failed at '{where_}': {how}\nevidence:\n{}",
            evidence.join("\n")
        ),
    }
}

/// True when any evidence line contains the needle.
fn evidence_has(evidence: &[String], needle: &str) -> bool {
    evidence.iter().any(|line| line.contains(needle))
}

/// V1: a run with a 2-turn budget against a worker needing 5 turns must
/// terminate in the budget-exhausted terminal state.
#[test]
fn tiny_budget_terminates_exhausted() {
    let evidence = require_pass(task_02::run_scenario(&ctx_for("default"), "default"));
    assert!(
        evidence_has(&evidence, "state=failed"),
        "expected terminal state=failed in evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        evidence_has(&evidence, "event budget.exhausted"),
        "expected a budget.exhausted event in evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        evidence_has(&evidence, "reason=budget exhausted"),
        "expected run.finished reason 'budget exhausted' in evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        evidence_has(&evidence, "used.turn=2"),
        "expected the consumption ledger in evidence:\n{}",
        evidence.join("\n")
    );
}

/// V2: the exhausted run must not carry a success verdict — no partial
/// "success" is reported.
#[test]
fn verdict_is_not_success() {
    let evidence = require_pass(task_02::run_scenario(&ctx_for("default"), "default"));
    assert!(
        evidence_has(&evidence, "verdict pass=false"),
        "expected verdict pass=false in evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        evidence_has(&evidence, "verdict is NOT success"),
        "expected an explicit non-success statement in evidence:\n{}",
        evidence.join("\n")
    );
}

/// A1: a spec with a zero budget must be rejected at creation, cleanly —
/// no run id, no exception, budget named in the error.
#[test]
fn zero_budget_rejected_at_creation() {
    let evidence = require_pass(task_02::run_scenario(
        &ctx_for("zero-budget"),
        "zero-budget",
    ));
    assert!(
        evidence_has(&evidence, "run_id=nil"),
        "expected no run id in evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        evidence_has(&evidence, "budget limit for turn must be a positive number"),
        "expected the budget rejection reason in evidence:\n{}",
        evidence.join("\n")
    );
}

/// A2: consuming past exhaustion is rejected and the run stays failed —
/// it must never resurrect as success.
#[test]
fn consume_after_exhaustion_rejected() {
    let evidence = require_pass(task_02::run_scenario(
        &ctx_for("consume-after-exhaustion"),
        "consume-after-exhaustion",
    ));
    assert!(
        evidence_has(&evidence, "post-exhaustion consume 1: REJECTED"),
        "expected post-exhaustion consumption to be rejected:\n{}",
        evidence.join("\n")
    );
    assert!(
        evidence_has(&evidence, "state after extra tick: failed"),
        "expected the run to stay failed:\n{}",
        evidence.join("\n")
    );
}
