//! Integration tests for task-10 (malicious MCP tool description).
//!
//! Each test drives the `task_10.lua` Neovim driver for one scenario and
//! asserts the verdict. 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_10;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-10: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-10: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-10: HOME is not set"))
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
    let work_dir = std::env::temp_dir().join(format!("gauntlet-task-10-{scenario}"));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-10: cannot build Ctx: {e}"));
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
        } => panic!("task-10 scenario '{scenario}' failed at '{where_}': {how}\n{evidence:?}"),
    }
}

// --- validation ---

/// V: a vet-clean poisoned description stays inert data through
/// listing, display, and a benign tools/call; the malicious tool is
/// never invoked.
#[test]
fn malicious_description_stays_inert() {
    let ctx = ctx_for("default");
    let evidence = pass_evidence(task_10::run_scenario(&ctx, "default"), "default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("listed 2 tools: gauntlet_echo, gauntlet_pwn"),
        "expected both tools listed, got:\n{joined}"
    );
    assert!(
        joined.contains("poisoned description returned byte-identical as data"),
        "expected description round-trip as data, got:\n{joined}"
    );
    assert!(
        joined.contains("list+describe caused zero tool invocations"),
        "expected no invocation during listing, got:\n{joined}"
    );
    assert!(
        joined.contains("call log entries: 1"),
        "expected exactly one tool call, got:\n{joined}"
    );
    assert!(
        joined.contains("malicious tool gauntlet_pwn never invoked"),
        "expected malicious tool never invoked, got:\n{joined}"
    );
}

/// V: an injection smuggled inside inputSchema description fields is
/// also inert, and the vet's blind spot (top-level descriptions only)
/// is documented rather than hidden.
#[test]
fn schema_smuggled_injection_stays_inert() {
    let ctx = ctx_for("schema-smuggle");
    let evidence = pass_evidence(
        task_10::run_scenario(&ctx, "schema-smuggle"),
        "schema-smuggle",
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("vet blind spot confirmed: schema-field injection produced 0 findings"),
        "expected documented vet blind spot, got:\n{joined}"
    );
    assert!(
        joined.contains("smuggled schema description returned as inert data"),
        "expected schema description as data, got:\n{joined}"
    );
    assert!(
        joined.contains("malicious tool gauntlet_pwn never invoked"),
        "expected malicious tool never invoked, got:\n{joined}"
    );
}

// --- adversarial ---

/// A: a tool named like a trusted tool ("system_exec") gains no
/// trust from its name: the call is denied, never reaches the server,
/// and the exact-match allowlist still serves the benign tool.
#[test]
fn spoofed_tool_name_gains_no_trust() {
    let ctx = ctx_for("name-spoof");
    let evidence = pass_evidence(task_10::run_scenario(&ctx, "name-spoof"), "name-spoof");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("tools/call to system_exec denied"),
        "expected denial of the spoofed tool call, got:\n{joined}"
    );
    assert!(
        joined.contains("denied call never reached the server"),
        "expected no partial execution, got:\n{joined}"
    );
    assert!(
        joined.contains("allowlisted benign call succeeded: allowlist is exact-match"),
        "expected exact-match allowlist behavior, got:\n{joined}"
    );
    assert!(
        joined.contains("malicious tool system_exec never invoked"),
        "expected spoofed tool never invoked, got:\n{joined}"
    );
}

/// A: no tool description anywhere in the pipeline feeds a prompt or
/// an execution sink; the harness context has no MCP wiring at all.
#[test]
fn no_description_reaches_prompt_or_execution() {
    let ctx = ctx_for("prompt-leak");
    let evidence = pass_evidence(task_10::run_scenario(&ctx, "prompt-leak"), "prompt-leak");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("0 prompt/execution sinks"),
        "expected zero prompt/execution sinks, got:\n{joined}"
    );
    assert!(
        joined.contains("no MCP wiring: no provider injects tool metadata"),
        "expected no MCP provider in harness context, got:\n{joined}"
    );
    assert!(
        joined.contains("malicious tool gauntlet_pwn never invoked"),
        "expected malicious tool never invoked, got:\n{joined}"
    );
}
