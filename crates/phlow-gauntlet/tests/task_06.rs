//! Integration tests for task-06 (ACP interop round-trip).
//!
//! Each test drives the `task_06.lua` Neovim driver for one scenario and
//! asserts the verdict. 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_06;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-06: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-06: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-06: HOME is not set"))
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
    let work_dir = std::env::temp_dir().join(format!("gauntlet-task-06-{scenario}"));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-06: cannot build Ctx: {e}"));
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
        } => panic!("task-06 scenario '{scenario}' failed at '{where_}': {how}\n{evidence:?}"),
    }
}

// --- validation ---

/// V: a full round-trip through the real ACP adapter — streamed
/// `session/update` chunk observed in the sink, run finishes completed.
#[test]
fn acp_round_trip_default() {
    let ctx = ctx_for("default");
    let evidence = pass_evidence(task_06::run_scenario(&ctx, "default"), "default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("session_update chunk observed: DEFAULT_TURN_CHUNK"),
        "expected the streamed chunk in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("model.completed outcome=completed"),
        "expected model.completed in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("run.finished state=completed"),
        "expected completed run.finished in evidence, got:\n{joined}"
    );
}

/// V: mid-session input via the adapter's real `send_input` reaches the
/// mock agent as a second prompt; its chunk is observed and the run
/// completes.
#[test]
fn acp_send_input_mid_session() {
    let ctx = ctx_for("send-input");
    let evidence = pass_evidence(task_06::run_scenario(&ctx, "send-input"), "send-input");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("first-turn chunk observed: FIRST_TURN_CHUNK"),
        "expected first-turn chunk in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("adapter send_input accepted mid-session input"),
        "expected send_input acceptance in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("second-turn chunk observed: SECOND_TURN_CHUNK"),
        "expected second-turn chunk in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("run.finished state=completed after the second turn"),
        "expected completion after the second turn, got:\n{joined}"
    );
}

// --- adversarial ---

/// A: naming an unregistered agent fails the run cleanly — no hang, the
/// failure is attributed to the unknown agent, the supervisor survives.
#[test]
fn acp_unknown_agent_clean_failure() {
    let ctx = ctx_for("unknown-agent");
    let evidence = pass_evidence(
        task_06::run_scenario(&ctx, "unknown-agent"),
        "unknown-agent",
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no hang: run reached state=failed"),
        "expected a clean failure without hanging, got:\n{joined}"
    );
    assert!(
        joined.contains("Unknown ACP agent: gauntlet-no-such-agent"),
        "expected the unknown-agent error in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("run.finished state=failed"),
        "expected failed run.finished in evidence, got:\n{joined}"
    );
}

/// A: garbage frames on the agent's stdout (non-JSON, JSON non-object,
/// unknown method, unknown response id) are swallowed by the transport;
/// the run still completes and no chunks are lost.
#[test]
fn acp_malformed_frame_no_crash() {
    let ctx = ctx_for("malformed-frame");
    let evidence = pass_evidence(
        task_06::run_scenario(&ctx, "malformed-frame"),
        "malformed-frame",
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("adversarial frames emitted by the mock: 5"),
        "expected proof that 5 garbage frames crossed the wire, got:\n{joined}"
    );
    assert!(
        joined.contains("session_update chunk survived the garbage: DEFAULT_TURN_CHUNK"),
        "expected the chunk to survive the garbage, got:\n{joined}"
    );
    assert!(
        joined.contains("run.finished state=completed"),
        "expected completion despite the garbage frames, got:\n{joined}"
    );
}
