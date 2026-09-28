//! task-117: A2A completion-integrity acceptance probe (nvim-lua, diver Fix 2).
//!
//! The diver defect: the real callback contract in `ai/a2a/tasks.lua`
//! (line 43) is `on_done? fun(task: A2aTask)` — one argument — but the
//! harness adapter declares `function(result, task_err)`
//! (adapters/a2a.lua line 63), so `task_err` is always nil and the recorded
//! outcome is always `'completed'`. Failed remote tasks are recorded as
//! completed.
//!
//! Acceptance-probe mode: the driver runs the REAL adapters/a2a.lua with a
//! stubbed `ai.a2a.tasks` transport (package.preload), invokes the captured
//! `on_done` with fabricated task tables in each terminal state, and banks
//! the acceptance criterion — the outcome must be derived from
//! `task.state` (completed→completed, canceled→cancelled,
//! failed/rejected→failed, anything else→failed, never completed).
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-117";
/// Human-readable name.
pub const NAME: &str = "a2a-completion-integrity";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_117.lua`.
pub const SCENARIOS: [&str; 4] = [
    "default",
    "completed-maps-completed",
    "rejected-canceled",
    "garbage-nil-double",
];

/// Attempt the task: run every driver scenario and report the first
/// failure, so the recorded outcome names the exact gap. All four must
/// pass for the task to pass.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    let mut evidence = Vec::new();
    for scenario in SCENARIOS {
        match run_scenario(ctx, scenario) {
            TaskOutcome::Pass { evidence: ev } => evidence.extend(ev),
            TaskOutcome::Fail {
                where_,
                how,
                evidence: ev,
            } => {
                evidence.extend(ev);
                return TaskOutcome::Fail {
                    where_,
                    how: format!("scenario {scenario:?}: {how}"),
                    evidence,
                };
            }
        }
    }
    TaskOutcome::Pass { evidence }
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_117.lua",
        "task-117",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
