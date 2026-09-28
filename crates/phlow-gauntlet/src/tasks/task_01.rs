//! task-01: fan-out/fan-in verdict aggregation (nvimlua).
//!
//! Drives `lua/gauntlet/task_01.lua` in headless Neovim. The driver
//! registers a fake `gauntlet` adapter at runtime against diver's real
//! `ai.harness` lifecycle, fans out 5 runs, polls with a bounded deadline
//! until all 5 are terminal, then fans in: terminal states are read from
//! the sink's `run.finished` events and the aggregate verdict is
//! pass iff all 5 completed. Scenarios are selected with the
//! `GAUNTLET_SCENARIO` env var: `default` (3 succeed, 1 fails, 1 cancelled
//! mid-run), `all-succeed`, `adapter-raises` (one adapter's `start()`
//! raises; the run must still terminate as failed and be counted).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-01";
/// Human-readable name.
pub const NAME: &str = "fan-out/fan-in verdict aggregation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task with the default scenario: 3 workers succeed, 1 fails,
/// 1 is cancelled mid-run. The correct aggregate is fail; the task passes
/// when the aggregation machinery observes 5 terminal runs, computes the
/// correct aggregate, and loses no run.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt the task with an explicit driver scenario: `"default"`,
/// `"all-succeed"`, or `"adapter-raises"`.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_01.lua",
        "task-01",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
