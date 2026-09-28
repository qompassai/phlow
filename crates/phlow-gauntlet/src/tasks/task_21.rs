//! task-21: saga compensating transactions (nvimlua).
//!
//! Recon task: the design asks for a saga coordinator (4-step saga with
//! reverse-order compensation) built on "real harness/workflow code". The
//! driver (`lua/gauntlet/task_21.lua`) proves the seam is ABSENT: diver's
//! `ai.harness` has a workflow *naming* layer (`registry.register_workflow`
//! / `get_workflow`: name → `{ adapter = ... }`) but no workflow *runner* —
//! `harness.run` never consults the registry and `spec.workflow` is a
//! validated label (`types.validate_run_spec`). Building a coordinator in
//! the driver would invent the seam the design forbids inventing.
//!
//! Every scenario therefore reports `fail` with `where = "seam"`: the
//! designed capability has no seam to drive. That is the honest finding —
//! an open design gap, not a driver error. The integration tests assert
//! the evidence documents the absence correctly.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-21";
/// Human-readable name.
pub const NAME: &str = "saga compensating transactions";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: recon the saga-coordinator seam (expected absent).
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"default"`, `"naming-layer"`, `"no-executor"`,
/// `"inert-def"`. Every scenario reports the seam absence; the tests
/// assert the evidence, not a pass.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_21.lua",
        "task-21",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
