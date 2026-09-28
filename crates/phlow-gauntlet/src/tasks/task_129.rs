//! task-129: approval-expiry one-shots acceptance probe (nvim-lua, diver
//! Phase-2 Decision 3).
//!
//! The design: an approval request schedules a one-shot at its expiry →
//! `wake(sup, now, 'approval')` → `approval.sweep_expired`; grant/deny
//! cancels the timer.
//!
//! Acceptance-probe mode: no one-shot is scheduled today (expiry is a
//! timestamp swept by `tick()`), so the adversarial gap scenario records
//! it with the exact hook location (`approval.request`); the other three
//! scenarios characterize today (tick-only sweep, no timer at request,
//! and the grant/deny-at-T-eps races resolved cleanly by tick
//! serialization). The gap record is the Phase-2 acceptance artifact.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-129";
/// Human-readable name.
pub const NAME: &str = "approval-expiry-one-shots";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_129.lua`.
pub const SCENARIOS: [&str; 4] = [
    "approval-expiry-via-tick",
    "no-approval-timer",
    "approval-one-shot-absent",
    "grant-before-expiry-no-double-decision",
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
        "task_129.lua",
        "task-129",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
