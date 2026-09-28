//! task-72: context handoff fidelity (nvimlua).
//!
//! The design asks what EXACTLY crosses the delegation boundary —
//! goal, constraints, budget remainder, tool allowlist — with authority
//! monotonically narrowing. Scenarios: default (the child run record's
//! actual field set: id/parent_id/root_id/workflow/adapter/workspace/
//! budget/extensions/acceptance — and `spec.goal`, though REQUIRED
//! non-empty by `validate_run_spec`, is dropped from the run table);
//! budget-not-split (the parent spends token budget; a child with no
//! explicit budget gets FULL defaults with zero used — two siblings
//! each get full defaults: no shared remainder, double-spend by
//! construction); adversarial: a 10MB `extensions.blob` is stored
//! silently (no bound, no explicit error, no truncation); adversarial:
//! the supervisor's policy is global (`policy.lua` allowlists live in
//! supervisor-wide rules) — the spawn spec has no allowlist field and
//! `spawn_child` performs no narrowing step, so child ⊆ parent is
//! unrepresentable and a widened child cannot be rejected at spawn.
//!
//! Seam mapping (verified, not invented): diver's spawn path
//! (`supervisor.spawn_child` → `supervisor.create`,
//! `lua/ai/harness/`) has NO handoff envelope. `create()` calls
//! `budget.new(spec.budget or DEFAULT_BUDGET_LIMITS)` — fresh full
//! budgets, never the parent's remainder. There is no per-run tool
//! allowlist and no spec size bound.
//!
//! Honest result: FAIL at `"seam"`. Every scenario's fail verdict
//! carries mechanism evidence (the run record's field list, the
//! sibling budget numbers, the round-tripped 10MB blob, the absent
//! allowlist field). Diver-owned finding: flagged, never fixed on
//! gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-72";
/// Human-readable name.
pub const NAME: &str = "context handoff fidelity";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "handoff-shape",
    "budget-not-split",
    "oversized-blob",
    "no-allowlist",
];

/// Attempt the task: handoff-shape facet first.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "handoff-shape")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"handoff-shape"`, `"budget-not-split"`,
/// `"oversized-blob"`, `"no-allowlist"`. Unknown names make the
/// driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_72.lua",
        "task-72",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
