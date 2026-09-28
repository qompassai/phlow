//! task-62: replanning on partial failure (nvimlua).
//!
//! Recon probe: the design asks for the planner's REPLAN entry point —
//! step 2 of 5 fails -> the planner emits a NEW plan for the remainder
//! (steps 3-5 with a workaround), completed steps are NOT re-executed,
//! and the replan is semantic (a step-2 failure invalidating step 1's
//! result correctly re-does step 1). "Locate the planner's replan entry
//! point; if the harness only supports resume, document the gap."
//!
//! The driver (`lua/gauntlet/task_62.lua`) scans the REAL ai tree — the
//! export tables of the harness modules plus a bounded whole-word
//! source-text scan of `lua/ai/harness` — for replan vocabulary
//! (`replan`, `revise_plan`, `new_plan`, `plan_v2`). It also reads the
//! actual bodies of `M.resume` and `M.retry_run` from `supervisor.lua`
//! to show what the recovery vocabulary really does.
//!
//! Honest result: the seam is ABSENT. The supervisor's recovery
//! vocabulary is `resume` (re-launch the SAME run_id — requires a
//! terminal run, same spec, same adapter; the body contains no plan
//! token) and `retry_run` (bounded same-run retry with backoff and an
//! attempt ceiling, transitioning the same run to `retry_wait`; the body
//! contains no plan token). Neither emits a new plan for the remainder;
//! neither is semantic about which completed steps stay valid. The rose
//! planner->coder->reviewer flow has role phases, not step plans 1..5,
//! and coder retry appends feedback to the same task. The design's
//! question is the finding: the harness only supports resume; the
//! replan gap is documented.
//!
//! Fail-closed: if replan machinery ever appears, the driver reports
//! `where = "recon"` (premise changed) instead of the seam absence.
//! Diver-owned finding: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-62";
/// Human-readable name.
pub const NAME: &str = "replanning on partial failure";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "replan-entry-point-scan",
    "resume-relaunches-same-run",
    "retry-keeps-same-run",
    "fail-closed-recon",
];

/// Attempt the task: probe diver's ai tree for a replan entry point.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "replan-entry-point-scan")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"replan-entry-point-scan"`,
/// `"resume-relaunches-same-run"`, `"retry-keeps-same-run"`,
/// `"fail-closed-recon"`. Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_62.lua",
        "task-62",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
