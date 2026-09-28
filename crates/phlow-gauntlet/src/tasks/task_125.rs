//! task-125: run selection TOCTOU for cancel/resume acceptance probe
//! (nvim-lua, diver Phase-2 Decision 2).
//!
//! The design: `:HarnessCancel` / `:HarnessResume` with no arg offer
//! `vim.ui.select` over eligible runs (live runs for cancel;
//! terminal-but-not-completed for resume). The TOCTOU core: a run that goes
//! terminal between listing and acting must make the act fail cleanly
//! (`'run is already terminal'`), with no corruption and no error event.
//! Selection is by run id, never by workflow name.
//!
//! Acceptance-probe mode: the picker commands do not exist today, so those
//! scenarios record the exact gap; the TOCTOU core and the unknown-run path
//! exercise `supervisor.cancel` directly and characterize today (both pass).
//! The gap records are the Phase-2 acceptance artifact.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-125";
/// Human-readable name.
pub const NAME: &str = "run-selection-toctou";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_125.lua`.
pub const SCENARIOS: [&str; 4] = [
    "cancel-picker-missing",
    "resume-picker-missing",
    "cancel-after-terminal",
    "cancel-unknown-run",
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
        "task_125.lua",
        "task-125",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
