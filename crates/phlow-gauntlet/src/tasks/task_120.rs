//! task-120: `M.run` failure-path legality acceptance probe (nvim-lua, diver Fix 6).
//!
//! The diver defect: `init.lua M.run` (line 83) calls
//! `supervisor.finish(..., 'failed')` for ANY failure — including failures
//! that happened BEFORE the created→queued transition inside `start_run`.
//! `finish` then attempts the illegal created→failed transition
//! (`types.TRANSITIONS.created` permits only queued/cancelled), emitting a
//! spurious `diagnostic.invalid_transition` and losing the intended failure
//! outcome.
//!
//! Acceptance-probe mode: the driver probes failures after the queued
//! transition (legal: reason preserved) and before it (illegal: no
//! diagnostic, run stays created), plus the not-set-up error and raising-
//! adapter containment, and banks the acceptance criterion: `finish` is
//! legal when the failure happened after queueing; no diagnostic is
//! emitted and no illegal transition attempted when the failure predates
//! the queued transition.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-120";
/// Human-readable name.
pub const NAME: &str = "run-failure-legality";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_120.lua`.
pub const SCENARIOS: [&str; 4] = [
    "default",
    "queued-failure",
    "pre-queued-failure",
    "not-set-up",
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
        "task_120.lua",
        "task-120",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
