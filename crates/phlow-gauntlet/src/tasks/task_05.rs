//! task-05: cancel/resume semantics (nvimlua).
//!
//! Drives diver's `ai.harness` through headless Neovim with a fake slow
//! adapter (see `lua/gauntlet/task_05.lua`). The default scenario cancels a
//! run mid-flight with reason `"gauntlet-test"`, asserts the `cancelled`
//! state and the recorded reason, resumes, and asserts the run reaches
//! `completed` exactly once. The named scenarios probe the rejection paths:
//! `"resume-completed"` (resume of a completed run is refused),
//! `"cancel-terminal"` (cancel of a terminal run is a clean error),
//! `"cancel-unknown"` (cancel of a bogus run id is a clean error).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-05";
/// Human-readable name.
pub const NAME: &str = "cancel/resume semantics";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: cancel mid-run, resume to completion.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"default"`, `"resume-completed"`, `"cancel-terminal"`,
/// `"cancel-unknown"`. Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_05.lua",
        "task-05",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
