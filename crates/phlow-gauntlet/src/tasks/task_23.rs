//! task-23: bounded dynamic fan-out (nvimlua).
//!
//! Drives diver's `ai.harness` supervisor spawn path
//! (`supervisor.spawn_child` → `supervisor.create`, see
//! `lua/gauntlet/task_23.lua`) with attacker-chosen spawn counts. The bound
//! under test is the supervisor's total-run cap (`RUNS_MAX = 256`):
//! `M.create` rejects with the explicit error `supervisor run bound
//! exceeded` once `sup.run_count` reaches `sup.runs_max`, and `run_count`
//! never decrements. There is no per-parent live-child bound and no
//! explicit depth bound — recursion is stopped only by the total-run cap.
//! The driver asserts the real mechanism and documents both facts.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-23";
/// Human-readable name.
pub const NAME: &str = "bounded dynamic fan-out";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: small-N fan-out spawns and completes.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"default"`, `"attacker-n"`, `"recursive"`. Unknown
/// names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_23.lua",
        "task-23",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
