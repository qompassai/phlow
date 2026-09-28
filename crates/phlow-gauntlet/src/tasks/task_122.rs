//! task-122: capability-based risk escalation acceptance probe (nvim-lua,
//! diver Phase-2 Decision 1, Fix 3 detail).
//!
//! The design: launch classifies risk from the adapter's pcall'd `probe()`
//! capabilities — `remote = true` -> risk `'network'`, else `'process'` —
//! and consults `policy.decide` with the built request before starting the
//! adapter. Broken probes fall back to `'network'` (fail-closed).
//!
//! Acceptance-probe mode: today launch builds no policy request at all, so
//! every scenario records that precise gap with scenario-specific evidence
//! (probe-call and decide-call spies around a real launch). The record is
//! the Phase-2 acceptance artifact.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-122";
/// Human-readable name.
pub const NAME: &str = "capability-risk-escalation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_122.lua`.
pub const SCENARIOS: [&str; 4] = [
    "remote-true",
    "remote-false",
    "probe-raises",
    "malformed-probe",
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
        "task_122.lua",
        "task-122",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
