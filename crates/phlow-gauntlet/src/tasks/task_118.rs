//! task-118: policy-launch-enforcement acceptance probe (nvim-lua, diver Fix 3).
//!
//! The diver defect: `supervisor.launch` (supervisor.lua line 182) goes
//! straight to `chosen.start` — `sup.policy` is stored but never consulted.
//! The fail-closed `policy.decide` (policy.lua lines 175-184: nil policy
//! denies) has no call sites, so the policy engine is decorative on the
//! launch path.
//!
//! Acceptance-probe mode: the driver probes four scenarios — deny blocks,
//! allow launches, nil-policy behavior, and the probe-attestation trust
//! boundary — and banks the acceptance criterion: launch must build the
//! risk request (probe caps via pcall, remote=true→"network" else
//! "process"), consult `decide` at launch time, and land denied runs in
//! `failed` with the policy reason recorded.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-118";
/// Human-readable name.
pub const NAME: &str = "policy-launch-enforcement";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_118.lua`.
pub const SCENARIOS: [&str; 4] = [
    "default",
    "allow-launches",
    "nil-policy",
    "no-classification",
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
        "task_118.lua",
        "task-118",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
