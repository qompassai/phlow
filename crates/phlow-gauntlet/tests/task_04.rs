//! task-04 integration tests: unknown adapter terminates invalid_adapter.
//!
//! Each test builds a [`Ctx`] from the environment (fail closed on missing
//! paths — never skip) and drives one driver scenario through headless
//! Neovim. 50/50 split: 2 validation + 2 adversarial.

use phlow_gauntlet::tasks::task_04;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build the task context for one scenario. Panics with a clear message
/// when the Neovim binary or diver's lua/ tree is missing or misconfigured.
fn test_ctx(scenario: &str) -> Ctx {
    let home = std::env::var("HOME").unwrap_or_else(|_| {
        panic!("task-04 test setup: HOME is not set and GAUNTLET_NVIM_BIN is not set")
    });
    let nvim_bin = std::env::var("GAUNTLET_NVIM_BIN")
        .unwrap_or_else(|_| format!("{home}/workspace/tools/neovim-nightly/bin/nvim"));
    let nvim_bin = PathBuf::from(nvim_bin);
    if !nvim_bin.is_file() {
        panic!(
            "task-04 test setup: nvim binary not found at {} (set GAUNTLET_NVIM_BIN)",
            nvim_bin.display()
        );
    }
    let diver_lua = std::env::var("GAUNTLET_DIVER_LUA")
        .unwrap_or_else(|_| format!("{home}/workspace/repos/diver/lua"));
    let diver_lua = PathBuf::from(diver_lua);
    if !diver_lua
        .join("ai")
        .join("harness")
        .join("init.lua")
        .is_file()
    {
        panic!(
            "task-04 test setup: diver harness not found under {} (set GAUNTLET_DIVER_LUA)",
            diver_lua.display()
        );
    }
    let work_dir = std::env::temp_dir().join(format!("gauntlet-task-04-{scenario}"));
    Ctx::new(nvim_bin, diver_lua, work_dir).unwrap_or_else(|e| {
        panic!("task-04 test setup: cannot build Ctx: {e}");
    })
}

/// Run one scenario; return the pass evidence, or panic quoting the fail.
fn scenario_evidence(scenario: &str) -> Vec<String> {
    let ctx = test_ctx(scenario);
    match task_04::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-04 scenario '{scenario}' reported FAIL\n  where: {where_}\n  how: {how}\n  evidence:\n    {}",
            evidence.join("\n    ")
        ),
    }
}

/// V: a bogus adapter name fails the run with an invalid_adapter diagnostic.
#[test]
fn unknown_adapter_fails_with_diagnostic() {
    let evidence = scenario_evidence("default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("invalid_adapter: unknown adapter: definitely-not-an-adapter"),
        "expected invalid_adapter diagnostic in evidence, got:\n{joined}"
    );
    assert!(
        joined.contains("run state=\"failed\""),
        "expected the run to end in state failed, got:\n{joined}"
    );
}

/// V: the diagnostic names the exact adapter that was rejected.
#[test]
fn diagnostic_names_the_adapter() {
    let evidence = scenario_evidence("default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains(
            "run.finished reason=\"invalid_adapter: unknown adapter: definitely-not-an-adapter\""
        ),
        "expected the finished reason to name the adapter, got:\n{joined}"
    );
    assert!(
        joined.contains("run() returned run_id=nil"),
        "expected run() to return no run id, got:\n{joined}"
    );
}

/// A: a path-traversal adapter name is rejected safely with no filesystem
/// writes outside the work dir.
#[test]
fn path_traversal_adapter_rejected_safely() {
    let evidence = scenario_evidence("path-traversal");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("invalid_adapter: unknown adapter: ../evil"),
        "expected invalid_adapter diagnostic for ../evil, got:\n{joined}"
    );
    assert!(
        joined.contains("unchanged=true"),
        "expected the parent-dir snapshot to be unchanged, got:\n{joined}"
    );
}

/// A: an empty adapter name is a clean validation error that creates no run.
#[test]
fn empty_adapter_rejected() {
    let evidence = scenario_evidence("empty");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("run spec.adapter must be a non-empty string when given"),
        "expected the adapter validation error, got:\n{joined}"
    );
    assert!(
        joined.contains("runs created=0"),
        "expected zero runs created, got:\n{joined}"
    );
}
