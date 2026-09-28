//! task-116: goal-propagation acceptance probe (nvim-lua, diver Fix 1).
//!
//! The diver defect: `types.validate_run_spec` (types.lua line 229) REQUIRES
//! `spec.goal`, but `supervisor.create` (supervisor.lua line 116) never copies
//! it into the run table — while `adapters/a2a.lua` (line 61) submits with
//! `message = run.goal`, i.e. nil. Every A2A run currently sends an empty
//! message to the remote agent.
//!
//! Acceptance-probe mode: Phase 2 is spec-only, so the driver records diver's
//! current behavior and banks the acceptance criterion — `run.goal` must
//! survive create byte-identical and the adapter must receive it intact at
//! start. Honest defect evidence is the success artifact; the driver never
//! fakes a pass.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-116";
/// Human-readable name.
pub const NAME: &str = "goal-propagation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_116.lua`.
pub const SCENARIOS: [&str; 4] = [
    "default",
    "launch-delivers-goal",
    "injection-pass-through",
    "unicode-whitespace",
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
        "task_116.lua",
        "task-116",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
