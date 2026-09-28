//! task-02: budget exhaustion fails closed (nvimlua).
//!
//! Drives `lua/gauntlet/task_02.lua` through headless Neovim. The driver
//! runs diver's `ai.harness` budget machinery: a run with a 2-turn budget
//! against a worker that needs 5 turns must terminate `failed` with reason
//! "budget exhausted" (terminal state per `types.TERMINAL_STATES`; the
//! distinct budget signal is the `budget.exhausted` event), and the
//! harness verdict must not be success. Extra scenarios ("zero-budget",
//! "consume-after-exhaustion") exercise creation-time rejection and
//! post-exhaustion consumption through the same driver via
//! `GAUNTLET_SCENARIO`.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-02";
/// Human-readable name.
pub const NAME: &str = "budget exhaustion fails closed";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task with the default scenario.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt the task under a named scenario.
///
/// Supported scenarios: `"default"`, `"zero-budget"`,
/// `"consume-after-exhaustion"`. The name is handed to the Lua driver
/// through `GAUNTLET_SCENARIO`; the driver owns its meaning.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_02.lua",
        "task-02",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
