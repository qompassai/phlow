//! task-123: `:HarnessRun` argument parsing contract acceptance probe
//! (nvim-lua, diver Phase-2 Decision 2).
//!
//! The design: hybrid — explicit args when given, prompts for the rest;
//! everything after `--` is the goal VERBATIM.
//!
//! Acceptance-probe mode: the command module does not exist today, so the
//! parsing scenarios record that exact gap and pin the contract; the
//! `goal-required` scenario characterizes the building block behind "never
//! creates a goal-less run" (`types.validate_run_spec`). The gap records
//! are the Phase-2 acceptance artifact.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-123";
/// Human-readable name.
pub const NAME: &str = "harnessrun-arg-parsing";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_123.lua`.
pub const SCENARIOS: [&str; 4] = [
    "parse-harnessrun",
    "goal-required",
    "double-dash-in-goal",
    "percent-hash-newline",
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
        "task_123.lua",
        "task-123",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
