//! Integration tests for task-05 (cancel/resume lifecycle semantics).
//!
//! Each test drives the `task_05.lua` Neovim driver for one scenario and
//! asserts the verdict. 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_05;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-05: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-05: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-05: HOME is not set"))
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
    let work_dir = std::env::temp_dir().join(format!("gauntlet-task-05-{scenario}"));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-05: cannot build Ctx: {e}"));
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
        } => panic!("task-05 scenario '{scenario}' failed at '{where_}': {how}\n{evidence:?}"),
    }
}

// --- validation ---

/// V: a run cancelled mid-flight lands in `cancelled` with the reason recorded.
#[test]
fn cancel_mid_run_yields_cancelled() {
    let ctx = ctx_for("default");
    let evidence = pass_evidence(task_05::run_scenario(&ctx, "default"), "default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("state after cancel(): cancelled"),
        "expected cancelled state in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("reason=gauntlet-test"),
        "expected recorded cancel reason in evidence, got:\n{joined}"
    );
}

/// V: a cancelled run resumes and reaches `completed` exactly once.
#[test]
fn resume_cancelled_completes() {
    let ctx = ctx_for("default");
    let evidence = pass_evidence(task_05::run_scenario(&ctx, "default"), "default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("attempt after resume: 2"),
        "expected resume to start attempt 2, got:\n{joined}"
    );
    assert!(
        joined.contains("run.finished events: total=2 completed=1"),
        "expected exactly one completion, got:\n{joined}"
    );
}

// --- adversarial ---

/// A: resuming a completed run is rejected; the run is not resurrected.
#[test]
fn resume_completed_rejected() {
    let ctx = ctx_for("resume-completed");
    let evidence = pass_evidence(
        task_05::run_scenario(&ctx, "resume-completed"),
        "resume-completed",
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("invalid transition completed -> queued"),
        "expected resume rejection in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("state still completed"),
        "expected completed state to survive, got:\n{joined}"
    );
}

/// A: cancelling a bogus run id is a clean error that creates nothing.
#[test]
fn cancel_unknown_id_clean_error() {
    let ctx = ctx_for("cancel-unknown");
    let evidence = pass_evidence(
        task_05::run_scenario(&ctx, "cancel-unknown"),
        "cancel-unknown",
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("unknown run: run-0000-bogus-id"),
        "expected unknown-run error in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("no run created"),
        "expected no run to be created, got:\n{joined}"
    );
}
