//! task-07: A2A task lifecycle (nvimlua).
//!
//! Drives diver's real `ai.harness` A2A adapter
//! (`lua/ai/harness/adapters/a2a.lua` -> `lua/ai/a2a/tasks.lua` ->
//! `lua/ai/a2a/client.lua`) against a mock A2A JSON-RPC peer implemented in
//! the driver on `vim.uv` TCP (see `lua/gauntlet/task_07.lua`). The default
//! scenario streams `working` -> `completed` through the real adapter and
//! verifies the wire shapes (lowercase `role: "user"`, `kind`
//! discriminators, kebab-case task states). The named scenarios probe the
//! failure paths: `"cancel"` (mid-stream cancel posts `tasks/cancel` and
//! settles `cancelled`), `"peer-dies"` (connection destroyed mid-stream
//! fails the run without hanging), `"bad-state"` (unknown state string is
//! ignored by the task state machine and the run still completes).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-07";
/// Human-readable name.
pub const NAME: &str = "A2A task lifecycle";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: full A2A lifecycle against the mock peer.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"default"`, `"cancel"`, `"peer-dies"`, `"bad-state"`.
/// Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_07.lua",
        "task-07",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
