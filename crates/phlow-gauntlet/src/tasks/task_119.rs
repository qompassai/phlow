//! task-119: resume-mutation-ordering acceptance probe (nvim-lua, diver Fix 5).
//!
//! The diver defect: `supervisor.resume` (supervisor.lua line 313) mutates
//! the run (attempt, generation, _terminal_emitted, handle) BEFORE
//! validating the queued transition. Resuming a completed run corrupts the
//! run table, then reports the invalid transition — the corruption is never
//! repaired.
//!
//! Acceptance-probe mode: the driver probes the resume path on failed,
//! completed, and running runs, and banks the acceptance criterion —
//! validate first (only terminal non-completed runs resume); a rejected
//! resume leaves the run table untouched; double resume advances attempt
//! monotonically without double-counting generation.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-119";
/// Human-readable name.
pub const NAME: &str = "resume-mutation-ordering";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_119.lua`.
pub const SCENARIOS: [&str; 4] = [
    "default",
    "generation-stale",
    "completed-resume",
    "running-rejection",
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
        "task_119.lua",
        "task-119",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
