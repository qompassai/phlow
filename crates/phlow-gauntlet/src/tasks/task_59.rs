//! task-59: approval scope binding (nvimlua).
//!
//! Two-part recon probe: the design asks for approval SCOPE binding —
//! an approval granted for action X, replayed for action Y, must be
//! rejected — with the approval record binding the exact action +
//! arguments and the EXECUTOR verifying the binding (proven by the
//! replay rejections).
//!
//! The driver (`lua/gauntlet/task_59.lua`) inspects the REAL seam:
//! `ai.harness.approval`'s record structure (behavioral: real request,
//! read the record back) and the harness tree for an executor that
//! consumes approval records (export-table scan + bounded source-text
//! scan for `require('ai.harness.approval')` consumers). It makes no
//! network calls and spawns no workers.
//!
//! Honest result: the seam is HALF-absent. The record structure binds
//! the action (tool/argv/paths/endpoints stored verbatim — the
//! structure half is real), but no executor exists in the harness to
//! verify the binding: the only approval consumer is
//! `supervisor.tick` → `sweep_expired` (expiry), and `policy.decide` /
//! `approval.request` have zero callers inside the repo (task-03's
//! declared gap, re-verified). The design's replay rejections cannot
//! be demonstrated against an executor that does not exist.
//!
//! Fail-closed: if executor-side binding verification ever appears,
//! the driver reports `where = "recon"` (premise changed) instead of
//! the seam absence.
//! Diver-owned finding: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-59";
/// Human-readable name.
pub const NAME: &str = "approval scope binding";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "record-binds-action",
    "binding-fields-verbatim",
    "no-executor-verifies",
    "replay-uncheckable",
];

/// Attempt the task: probe the approval record structure and the
/// (absent) executor verification.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "record-binds-action")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"record-binds-action"`,
/// `"binding-fields-verbatim"`, `"no-executor-verifies"`,
/// `"replay-uncheckable"`. Unknown names make the driver report
/// failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_59.lua",
        "task-59",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
