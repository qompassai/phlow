//! task-03: approval gate blocks unapproved tool use (nvimlua).
//!
//! Drives `lua/gauntlet/task_03.lua` in headless Neovim through the real
//! harness enforcement seam: `policy.decide` (the one authorization decision
//! point) plus the `approval` queue (request / decide / expiry). Scenarios:
//! "default" (approver grants -> tool proceeds), "no-approver" (nobody
//! decides -> blocked, approval expires to denied), "denied" (approver
//! denies -> blocked), "unknown-tool" (no matching rule -> policy denies,
//! no approval requested).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-03";
/// Human-readable name.
pub const NAME: &str = "approval gate blocks unapproved tool use";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task (the "default" scenario: an approver grants).
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one approval-gate scenario via the headless-Neovim driver.
///
/// `scenario` is passed through as `GAUNTLET_SCENARIO`; the driver owns its
/// meaning. Rejected inputs (bad scenario names) fail inside the driver
/// verdict, never silently.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_03.lua",
        "task-03",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
