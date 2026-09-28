//! task-124: prompt fallback and abort atomicity acceptance probe (nvim-lua,
//! diver Phase-2 Decision 2).
//!
//! The design: missing pieces fall back to `vim.ui.select` (adapter, from
//! the registry) / `vim.ui.input` (workflow, goal), in that order;
//! aborting any prompt aborts the WHOLE command — zero runs created, zero
//! events appended. A whitespace-only goal is treated as abort, not as a
//! goal (the trim rule must live in the command layer, since
//! `validate_run_spec` accepts `'   '`).
//!
//! Acceptance-probe mode: the command module does not exist today, so every
//! scenario records that exact gap with the full acceptance contract. The
//! records are the Phase-2 acceptance artifact; when Phase 2 ships the
//! command, the driver becomes its executable spec (scripted `vim.ui`
//! sequences, counting runs and sink events).
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-124";
/// Human-readable name.
pub const NAME: &str = "prompt-fallback-abort-atomicity";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_124.lua`.
pub const SCENARIOS: [&str; 4] = [
    "bare-harnessrun",
    "partial-args",
    "abort-atomicity",
    "whitespace-goal",
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
        "task_124.lua",
        "task-124",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
