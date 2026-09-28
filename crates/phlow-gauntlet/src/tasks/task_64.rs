//! task-64: deterministic tool selection (nvimlua).
//!
//! The design asks for the tool-selection / dispatch seam: two tools both
//! match an intent -> the choice must be deterministic and explained.
//! Scenarios: default (one matches -> selected); ambiguity (two match ->
//! the documented precedence rule picks one, and the rationale is
//! logged); adversarial: the same ambiguous input 100 times -> the same
//! choice every time (no hash-order or timing dependence).
//!
//! Seam mapping (documented, not invented): diver has no intent-string
//! matcher — dispatch is by exact name everywhere (`rose/tools.lua`
//! `M.call`, `mcp/tools.lua` `describe`, the registry's `get_tool`). The
//! one capability-based SELECTION seam is
//! `ai.harness.adapter.negotiate`: "Choose the first adapter (in sorted
//! name order) whose probed capabilities satisfy every requested need."
//! Intent ~= requested capability needs; tools ~= adapters. The real
//! supervisor uses it at launch (`supervisor.lua` calls
//! `adapter.negotiate(adapters, { cancellation = true })`).
//!
//! The driver (`lua/gauntlet/task_64.lua`) exercises the REAL
//! `ai.harness.adapter.negotiate` with mock adapters: single-match
//! selection, ambiguity resolved by sorted-name-first regardless of
//! registration order, a 100-run stability battery with alternating
//! insertion order, and a flapping-probe boundary case showing
//! determinism is conditional on probe outcomes (probe failure becomes
//! "unavailable", never raises — an input change, not nondeterminism).
//!
//! Honest result: PASS on the core dimension, with two banked caveats.
//! Selection IS a pure function of (needs, adapter set, probe outcomes):
//! names are `table.sort`'ed before first-match, so Lua hash order can
//! never leak in; the 100-run battery proves it. The design's
//! "explained" half is met documentarily (the rule is in the docstring
//! and the registry header) but not per-selection: `negotiate` returns
//! only the adapter, no rationale record is produced or logged (caveat
//! C1, banked). The "precedence config" is likewise absent: precedence
//! is hardcoded sorted-name order, not a config (caveat C2, banked).
//! Diver-owned caveats: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-64";
/// Human-readable name.
pub const NAME: &str = "deterministic tool selection";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "single-match",
    "ambiguity-sorted-name",
    "hundred-run-stability",
    "probe-flap-boundary",
];

/// Attempt the task: drive the real `ai.harness.adapter.negotiate`.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "single-match")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"single-match"`, `"ambiguity-sorted-name"`,
/// `"hundred-run-stability"`, `"probe-flap-boundary"`. Unknown names
/// make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_64.lua",
        "task-64",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
