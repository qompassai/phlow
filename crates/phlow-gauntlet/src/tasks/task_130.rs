//! task-130: idle-loop quietness — the phone-battery acceptance test
//! (nvim-lua, diver Phase-2 Decision 3).
//!
//! The design justification for "no poll" over 250ms/1s: a polling
//! supervisor wakes the event loop forever; an event-driven one sleeps.
//!
//! Acceptance-probe mode: the loop is quiet today by absence (no
//! supervision exists without explicit `tick()`), so the two validation
//! scenarios characterize the quietness (10s idle window with zero
//! wakeups; 100 settled runs back at the handle baseline) while the two
//! adversarial scenarios record the exact gap (no pending handle for a
//! far-future deadline; no timer teardown path). The gap records are the
//! Phase-2 acceptance artifact.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-130";
/// Human-readable name.
pub const NAME: &str = "idle-loop-quietness";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_130.lua`.
pub const SCENARIOS: [&str; 4] = [
    "idle-window-zero-activity",
    "settled-runs-baseline-handles",
    "deadline-handle-absent",
    "timer-teardown-absent",
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
        "task_130.lua",
        "task-130",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
