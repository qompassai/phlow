//! Integration tests for task-03: approval gate blocks unapproved tool use.
//!
//! Fail closed: missing nvim binary or diver lua dir panics with a clear
//! message — a test that cannot reach the real harness is a failure, never
//! a skip. 50/50 split: 2 validation + 2 adversarial.

use phlow_gauntlet::tasks::task_03;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` for one scenario. Panics (fail closed) when the required
/// paths are missing instead of skipping.
fn gauntlet_ctx(scenario: &str) -> Ctx {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/hatch".to_string());
    let nvim_default = format!("{home}/workspace/tools/neovim-nightly/bin/nvim");
    let diver_default = format!("{home}/workspace/repos/diver/lua");
    let nvim_bin = PathBuf::from(std::env::var("GAUNTLET_NVIM_BIN").unwrap_or(nvim_default));
    let diver_lua = PathBuf::from(std::env::var("GAUNTLET_DIVER_LUA").unwrap_or(diver_default));
    if !nvim_bin.is_file() {
        panic!(
            "task-03 test setup: nvim binary not found at {} (set GAUNTLET_NVIM_BIN)",
            nvim_bin.display()
        );
    }
    if !diver_lua.is_dir() {
        panic!(
            "task-03 test setup: diver lua dir not found at {} (set GAUNTLET_DIVER_LUA)",
            diver_lua.display()
        );
    }
    let work_dir = std::env::temp_dir().join(format!("gauntlet-task-03-{scenario}"));
    Ctx::new(nvim_bin, diver_lua, work_dir).unwrap_or_else(|e| {
        panic!("task-03 test setup: invalid Ctx: {e}");
    })
}

/// The driver verdict must be Pass and must carry evidence.
fn assert_driver_pass(scenario: &str, outcome: TaskOutcome) {
    match outcome {
        TaskOutcome::Pass { evidence } => {
            assert!(
                !evidence.is_empty(),
                "task-03/{scenario}: pass verdict carried no evidence"
            );
        }
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            panic!(
                "task-03/{scenario}: expected pass, driver reported fail\nwhere: {where_}\nhow: {how}\nevidence: {evidence:?}"
            );
        }
    }
}

/// V1: an approver grants -> the approved tool call demonstrably proceeds.
#[test]
fn approved_tool_call_proceeds() {
    let ctx = gauntlet_ctx("default");
    let outcome = task_03::run_scenario(&ctx, "default");
    assert_driver_pass("default", outcome);
}

/// V2: no approver configured -> the call is BLOCKED and never executes.
#[test]
fn no_approver_blocks_call() {
    let ctx = gauntlet_ctx("no-approver");
    let outcome = task_03::run_scenario(&ctx, "no-approver");
    assert_driver_pass("no-approver", outcome);
}

/// A1: an approver denies -> the denial is handled (blocked, recorded).
#[test]
fn denied_approval_handled() {
    let ctx = gauntlet_ctx("denied");
    let outcome = task_03::run_scenario(&ctx, "denied");
    assert_driver_pass("denied", outcome);
}

/// A2: approval requested for an unknown tool -> rejected cleanly by policy.
#[test]
fn unknown_tool_rejected() {
    let ctx = gauntlet_ctx("unknown-tool");
    let outcome = task_03::run_scenario(&ctx, "unknown-tool");
    assert_driver_pass("unknown-tool", outcome);
}
