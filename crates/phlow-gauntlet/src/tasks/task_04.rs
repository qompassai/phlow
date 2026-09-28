//! task-04: unknown adapter terminates invalid_adapter (nvimlua).
//!
//! Regression probe of diver's agent harness (`ai.harness`): a bogus
//! adapter name must fail the run with an `invalid_adapter:` diagnostic —
//! no panic, no hang, no silent success. The attempt is driven by the
//! headless-Neovim Lua driver `lua/gauntlet/task_04.lua`; this module only
//! selects the scenario and forwards the verdict.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-04";
/// Human-readable name.
pub const NAME: &str = "unknown adapter terminates invalid_adapter";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task with the default scenario (`definitely-not-an-adapter`).
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one named scenario: `default`, `path-traversal`, `empty`, or
/// `phlow-stub`. The driver owns scenario validation; an unknown scenario
/// becomes a driver fail verdict, never a Rust-side panic.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_04.lua",
        "task-04",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
