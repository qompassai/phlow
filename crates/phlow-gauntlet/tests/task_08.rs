//! Integration tests for task-08 (MCP stdio tool bridging).
//!
//! Each test drives the `task_08.lua` Neovim driver for one scenario and
//! asserts the verdict. 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_08;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-08: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-08: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-08: HOME is not set"))
}

/// Build a `Ctx` for one scenario with its own scratch directory.
///
/// The directory is suffixed with the process id: the driver's MCP
/// registry and security allowlist persist under the work dir, so a
/// fixed path would see a previous run's registrations ("server already
/// registered") and fail. Fresh dir per test-process run keeps the
/// tests hermetic and re-runnable.
fn ctx_for(scenario: &str, timeout_secs: u64) -> Ctx {
    let nvim_bin = required_dir(
        "GAUNTLET_NVIM_BIN",
        &format!("{}/workspace/tools/neovim-nightly/bin/nvim", home_dir()),
    );
    let diver_lua = required_dir(
        "GAUNTLET_DIVER_LUA",
        &format!("{}/workspace/repos/diver/lua", home_dir()),
    );
    let work_dir = std::env::temp_dir().join(format!(
        "gauntlet-task-08-{scenario}-{}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-08: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(timeout_secs);
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
        } => panic!("task-08 scenario '{scenario}' failed at '{where_}': {how}\n{evidence:?}"),
    }
}

// --- validation ---

/// V: the real-server probe pins the initialize interop gap, then the mock
/// server answers tools/list with its three tools and tools/call
/// gauntlet_add returns "42" through the real adapter.
#[test]
fn default_real_server_round_trip() {
    let ctx = ctx_for("default", 120);
    let evidence = pass_evidence(task_08::run_scenario(&ctx, "default"), "default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("real-server probe: run failed as expected"),
        "expected the real-server probe in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("Initialize requires capabilities and clientInfo"),
        "expected the pinned interop error in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("names=gauntlet_add,gauntlet_blob,gauntlet_echo"),
        "expected the mock's three tools in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("tools/call gauntlet_add{a=40,b=2} -> \"42\""),
        "expected the gauntlet_add round trip in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("adapter session completed: model.completed outcome=completed"),
        "expected the adapter session to complete the run, got:\n{joined}"
    );
    assert!(
        joined.contains("tool result recorded into run"),
        "expected the tool result to flow back into the run, got:\n{joined}"
    );
}

/// V: schema violations surface as typed Invalid params errors and the run
/// survives them.
#[test]
fn bad_args_typed_errors() {
    let ctx = ctx_for("bad-args", 120);
    let evidence = pass_evidence(task_08::run_scenario(&ctx, "bad-args"), "bad-args");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("missing required argument"),
        "expected missing-arg typed error in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("must be an integer"),
        "expected wrong-type typed error in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("Unknown tool"),
        "expected unknown-tool typed error in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("run still completed after three typed errors"),
        "expected the run to survive the error path, got:\n{joined}"
    );
}

// --- adversarial ---

/// A: a server that exits mid-call produces a typed process-exit error;
/// the pending request resolves instead of hanging.
#[test]
fn server_dies_no_hang() {
    let ctx = ctx_for("server-dies", 120);
    let evidence = pass_evidence(task_08::run_scenario(&ctx, "server-dies"), "server-dies");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("server exited"),
        "expected typed server-exited error in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("no hang"),
        "expected the no-hang observation in evidence, got:\n{joined}"
    );
}

/// A: a ~10 MiB single-line response past the client's 8 MiB frame cap is
/// dropped; the call times out with a typed error, memory growth stays
/// bounded, and the session remains usable.
#[test]
fn oversize_frame_bounded() {
    let ctx = ctx_for("oversize", 180);
    let evidence = pass_evidence(task_08::run_scenario(&ctx, "oversize"), "oversize");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("10 MiB frame dropped"),
        "expected the dropped-frame observation in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("timed out"),
        "expected the typed timeout in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("lua memory:"),
        "expected the memory measurement in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("session survived"),
        "expected the session-survival check in evidence, got:\n{joined}"
    );
}
