//! task-61: plan schema validation (nvimlua).
//!
//! Recon probe: the design asks for the planner -> executor handoff seam
//! — the planner's output validated against a schema BEFORE anything
//! executes, with the structural pass criterion that the executor's
//! input type IS the validated plan ("locate the plan representation;
//! the harness `workflow?` spec is a candidate").
//!
//! The driver (`lua/gauntlet/task_61.lua`) scans the REAL ai tree — the
//! export tables of the harness modules plus a bounded source-text scan
//! of `lua/ai` — for plan-validation vocabulary (`validate_plan`,
//! `plan_schema`, `plan_validator`, `schema_violation`). It also drives
//! the two closest "plan" candidates behaviorally: `ai.rose`'s `M.plan`
//! and the harness registry's `register_workflow`.
//!
//! Honest result: the seam is ABSENT. `ai.rose`'s `M.plan` produces plan
//! TEXT for humans ("Output ONLY the plan as plain text",
//! `lua/ai/rose/init.lua`) — there is no machine plan type, hence no
//! plan schema and no executor input type that IS a validated plan. The
//! registry's `register_workflow` accepts arbitrary def shapes (only
//! `adapter` is validated): a malformed "plan" (missing step, unknown
//! tool, cyclic dependency) is accepted verbatim, so it cannot be
//! rejected "with the schema violation named" — there is no schema. The
//! design's structural pass criterion is unmeetable: there is no plan
//! type at all.
//!
//! Fail-closed: if plan-validation machinery ever appears, the driver
//! reports `where = "recon"` (premise changed) instead of the seam
//! absence. Diver-owned finding: flagged, never fixed on gauntlet
//! authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-61";
/// Human-readable name.
pub const NAME: &str = "plan schema validation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "plan-schema-scan",
    "workflow-def-has-no-plan-schema",
    "malformed-plan-cannot-be-rejected",
    "fail-closed-recon",
];

/// Attempt the task: probe diver's ai tree for a plan-schema validator.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "plan-schema-scan")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"plan-schema-scan"`,
/// `"workflow-def-has-no-plan-schema"`,
/// `"malformed-plan-cannot-be-rejected"`, `"fail-closed-recon"`.
/// Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_61.lua",
        "task-61",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
