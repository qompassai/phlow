//! task-127: deadline one-shots and handle hygiene acceptance probe
//! (nvim-lua, diver Phase-2 Decision 3).
//!
//! The design: run create schedules a one-shot `vim.uv` timer at
//! `deadline_ns` → `wake(sup, now, 'deadline')`; the handle is tracked in
//! `run._timers` and cancelled on terminal entry inside `transition()`
//! (the airtight place — every terminal path goes through it).
//!
//! Acceptance-probe mode: no one-shot is scheduled today (the deadline is
//! a timestamp compared inside `tick()`), so the two adversarial
//! scenarios record the exact gap (no one-shot at create; `transition()`
//! cancels nothing); the two validation scenarios characterize today
//! (tick-only deadline enforcement, no timer scheduled at create). The gap
//! records are the Phase-2 acceptance artifact.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-127";
/// Human-readable name.
pub const NAME: &str = "deadline-one-shots-handle-hygiene";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_127.lua`.
pub const SCENARIOS: [&str; 4] = [
    "deadline-fires-via-tick",
    "no-one-shot-at-create",
    "deadline-one-shot-absent",
    "terminal-entry-no-timer-cleanup",
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
        "task_127.lua",
        "task-127",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
