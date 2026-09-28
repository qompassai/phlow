//! task-128: retry one-shot lifecycle acceptance probe (nvim-lua, diver
//! Phase-2 Decision 3).
//!
//! The design: `retry_run` schedules a one-shot at `retry_at_ns` →
//! `wake(sup, now, 'retry')` → re-queue and relaunch; cancel during
//! `retry_wait` cancels the timer.
//!
//! Acceptance-probe mode: no one-shot is scheduled today (the retry is a
//! timestamp promoted by `tick()`), so the adversarial gap scenario
//! records it; the other three scenarios characterize today (tick-only
//! promotion with exactly-once attempt increment, the ceiling refusal,
//! and cancel-during-retry-wait with no phantom relaunch). The gap record
//! is the Phase-2 acceptance artifact.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-128";
/// Human-readable name.
pub const NAME: &str = "retry-one-shot-lifecycle";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_128.lua`.
pub const SCENARIOS: [&str; 4] = [
    "retry-promotes-via-tick",
    "retry-ceiling-refusal",
    "retry-one-shot-absent",
    "cancel-during-retry-wait",
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
        "task_128.lua",
        "task-128",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
