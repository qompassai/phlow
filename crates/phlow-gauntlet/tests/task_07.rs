//! Integration tests for task-07 (A2A task lifecycle).
//!
//! Each test drives the `task_07.lua` Neovim driver for one scenario and
//! asserts the verdict. 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_07;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-07: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-07: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-07: HOME is not set"))
}

/// Build a `Ctx` for one scenario with its own scratch directory.
fn ctx_for(scenario: &str) -> Ctx {
    let nvim_bin = required_dir(
        "GAUNTLET_NVIM_BIN",
        &format!("{}/workspace/tools/neovim-nightly/bin/nvim", home_dir()),
    );
    let diver_lua = required_dir(
        "GAUNTLET_DIVER_LUA",
        &format!("{}/workspace/repos/diver/lua", home_dir()),
    );
    let work_dir = std::env::temp_dir().join(format!("gauntlet-task-07-{scenario}"));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-07: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Unwrap a passing verdict into its evidence, or panic with the failure.
fn pass_evidence(outcome: TaskOutcome, scenario: &str) -> Vec<String> {
    match outcome {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-07 scenario '{scenario}' failed at '{where_}': {how}\n{evidence:?}"),
    }
}

// --- validation ---

/// V: the full lifecycle completes through the real adapter, with the
/// kebab-case task states observed on the wire and the wire shapes verified.
#[test]
fn default_lifecycle_reaches_completed() {
    let ctx = ctx_for("default");
    let evidence = pass_evidence(task_07::run_scenario(&ctx, "default"), "default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("rpc method=message/stream"),
        "expected message/stream on the wire, got:\n{joined}"
    );
    assert!(
        joined.contains("kebab-case states on the wire: working -> completed"),
        "expected quoted kebab-case states, got:\n{joined}"
    );
    assert!(
        joined.contains("wire-shape-ok"),
        "expected wire shape verification, got:\n{joined}"
    );
    assert!(
        joined.contains("role=user"),
        "expected lowercase role=user in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("verdict recorded: model.completed outcome=completed"),
        "expected completion verdict, got:\n{joined}"
    );
}

/// V: cancelling mid-stream posts tasks/cancel to the peer and settles the
/// run as cancelled, with no stale completion leaking through.
#[test]
fn cancel_sends_remote_tasks_cancel() {
    let ctx = ctx_for("cancel");
    let evidence = pass_evidence(task_07::run_scenario(&ctx, "cancel"), "cancel");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("state after cancel(): cancelled"),
        "expected cancelled state, got:\n{joined}"
    );
    assert!(
        joined.contains("rpc tasks/cancel"),
        "expected tasks/cancel on the wire, got:\n{joined}"
    );
    assert!(
        joined.contains("state=cancelled"),
        "expected run.finished cancelled, got:\n{joined}"
    );
}

// --- adversarial ---

/// A: the peer dying mid-stream is detected by the REAL A2A task layer
/// (curl exit 18 on the truncated stream -> local task failed, no hang), but
/// the harness a2a adapter misreports it: `adapters/a2a.lua` registers
/// `on_done = function(result, task_err)` while `ai.a2a.tasks` documents and
/// calls `on_done(task)`, so `task_err` is always nil and every failed task
/// is recorded as completed. The scenario verdict is therefore FAIL with the
/// exact mechanism; this test pins that diagnosis so the bug cannot slip by
/// silently. When the adapter is fixed, this test goes red on purpose.
#[test]
fn peer_death_exposes_adapter_misreport() {
    let ctx = ctx_for("peer-dies");
    let (where_, how, evidence) = match task_07::run_scenario(&ctx, "peer-dies") {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-07 peer-dies unexpectedly passed; expected the adapter-bug \
             diagnosis, got:\n{}",
            evidence.join("\n")
        ),
    };
    let joined = evidence.join("\n");
    assert_eq!(
        where_, "peer-dies",
        "expected failure at peer-dies, got '{where_}'"
    );
    assert!(
        how.contains("on_done"),
        "expected on_done diagnosis in how, got: {how}"
    );
    assert!(
        joined.contains("local a2a task state: failed"),
        "expected A2A-layer failure in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("stream ended"),
        "expected curl transfer error in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("outcome=completed"),
        "expected misreported completed outcome in evidence, got:\n{joined}"
    );
}

/// A: an unknown task state string from the peer is ignored by the task
/// state machine; the run still completes uncorrupted.
#[test]
fn unknown_task_state_is_ignored() {
    let ctx = ctx_for("bad-state");
    let evidence = pass_evidence(task_07::run_scenario(&ctx, "bad-state"), "bad-state");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("wire carried unknown state string: frobnicate"),
        "expected frobnicate on the wire, got:\n{joined}"
    );
    assert!(
        joined.contains("run completed uncorrupted"),
        "expected uncorrupted completion, got:\n{joined}"
    );
}
