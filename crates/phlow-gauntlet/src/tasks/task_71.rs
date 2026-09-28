//! task-71: delegation depth attribution (nvimlua).
//!
//! The design asks for depth of the LIVE spawn tree attributed by the
//! supervisor from its own run ancestry — never self-reported by the
//! child. Scenarios: default (parent→child→grandchild; the ancestry
//! walk yields 0/1/2 from the supervisor's own parent_id tree);
//! depth-unbounded (a 30-deep chain spawns with zero resistance — no
//! `delegation_depth_exceeded`, no named depth constant); adversarial:
//! the child forges `extensions.claimed_depth = 0` (the lie passes
//! through unread) and a spawn names a FOREIGN parent_id (an unrelated
//! live run — accepted silently); adversarial: a source scan of the
//! loaded supervisor.lua finds zero "depth" mentions and a chain to the
//! total-run cap is refused only with 'supervisor run bound exceeded'.
//!
//! Seam mapping (verified, not invented): diver's supervisor
//! (`lua/ai/harness/supervisor.lua`) keeps a run tree
//! (`parent_id`/`root_id`, `parent.children`) but `M.create` /
//! `M.spawn_child` never walk it, compute no depth, enforce no depth
//! bound, and verify nothing about the asserted parent_id beyond
//! "exists and non-terminal". The design's "supervisor recomputes
//! depth from its own run tree and rejects the lie" has no
//! implementation.
//!
//! Honest result: FAIL at `"seam"`. Every scenario's fail verdict
//! carries mechanism evidence (the walked depths, the accepted
//! forgeries, the source scan). Diver-owned finding: flagged, never
//! fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-71";
/// Human-readable name.
pub const NAME: &str = "delegation depth attribution";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "chain-depth",
    "depth-unbounded",
    "forged-depth",
    "no-depth-error",
];

/// Attempt the task: chain-depth facet first.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "chain-depth")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"chain-depth"`, `"depth-unbounded"`,
/// `"forged-depth"`, `"no-depth-error"`. Unknown names make the
/// driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_71.lua",
        "task-71",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
