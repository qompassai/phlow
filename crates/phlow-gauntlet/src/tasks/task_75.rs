//! task-75: delegation cycle detection (nvimlua).
//!
//! The design asks that cycles in the delegation graph be rejected
//! with a typed error — a depth bound alone cannot catch a 2-node
//! cycle (it never gets deep; it spins). Scenarios: default (a linear
//! chain of 5 works; the O(depth) ancestry walk yields 0..4 with no
//! false positives); walk-sound (a branching tree of 6 nodes — every
//! ancestry set exact, validating the walk the design demands);
//! adversarial: the "escalation loop" — B (child of A) delegates back
//! UP to its ancestor A under a different workflow name — is ACCEPTED
//! silently, not rejected by identity; adversarial: a source scan of
//! the loaded supervisor.lua finds the only "cycle" substring inside
//! the word "lifecycle" (header comment) and no
//! `delegation_cycle` string, and a spawn naming an unrelated live run
//! as parent is accepted — the ancestry set is never consulted at
//! spawn.
//!
//! Seam mapping (verified, not invented): diver's spawn path
//! (`supervisor.spawn_child` → `supervisor.create`,
//! `lua/ai/harness/`) performs no ancestry-membership check. Strict
//! graph cycles are structurally unrepresentable (a parent_id must name
//! an already-existing run; ids are minted fresh at create), but that
//! is construction, not a check — and the design's checkable cases
//! (escalation loops, upward delegation) meet no check.
//!
//! Honest result: FAIL at `"seam"`. Every scenario's fail verdict
//! carries mechanism evidence (the walked ancestry sets, the accepted
//! escalation, the source scan). Diver-owned finding: flagged, never
//! fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-75";
/// Human-readable name.
pub const NAME: &str = "delegation cycle detection";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "linear-chain",
    "walk-sound",
    "escalation-accepted",
    "no-cycle-machinery",
];

/// Attempt the task: linear-chain facet first.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "linear-chain")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"linear-chain"`, `"walk-sound"`,
/// `"escalation-accepted"`, `"no-cycle-machinery"`. Unknown names make
/// the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_75.lua",
        "task-75",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
