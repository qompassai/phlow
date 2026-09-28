//! task-121: deny-all default + explicit opt-in acceptance probe (nvim-lua,
//! diver Phase-2 Decision 1).
//!
//! The decision: diver ships `policy = { default = 'deny', rules = {} }`;
//! `policy_example.lua` (allow observe + local_reversible) is shipped
//! BESIDE it, never loaded implicitly — enabling it is one explicit line,
//! because `local_reversible` is currently the adapter's unverified claim.
//!
//! Acceptance-probe mode: the first three scenarios characterize today's
//! behavior (fresh setup denies; `decide` fail-closes; the setup path never
//! loads the example implicitly) and the fourth records the exact gap —
//! `ai.harness.policy_example` does not exist. That record is the Phase-2
//! acceptance artifact.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-121";
/// Human-readable name.
pub const NAME: &str = "deny-all-default-opt-in";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_121.lua`.
pub const SCENARIOS: [&str; 4] = [
    "default",
    "decide-fail-closed",
    "no-implicit-example",
    "opt-in-absent",
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
        "task_121.lua",
        "task-121",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
