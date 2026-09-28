//! task-73: subagent failure containment (nvimlua).
//!
//! The design asks that a failed subagent be QUARANTINED: its subtree
//! reclaimed, its partial outputs quarantined from the parent context,
//! and its verdict claims verified against evidence — never trusted as
//! strings. Scenarios: default (a failed child is recorded with its
//! cause chain while siblings are unaffected — the recording half
//! works); adversarial: a child hangs past its 50ms deadline with a
//! live grandchild — `tick()`'s `finish()` refuses with 'parent run
//! owns live children', no subtree-kill API exists (zero "subtree"
//! mentions in supervisor.lua), and `M.cancel` on the child orphans the
//! grandchild; adversarial: `finish(child, 'completed', 'subagent says
//! so')` with zero evidence artifacts lands in `completed`,
//! indistinguishable from a verified completion — no
//! `subagent_unverified` outcome exists and `finish()` never calls
//! `verdict.evaluate`; adversarial: the hostile adapter's
//! `model.completed` payload carrying an injection string is merged
//! verbatim into the run.finished record by `drain_completions` — no
//! quarantine, no sanitization.
//!
//! Seam mapping (verified, not invented): diver's supervisor
//! (`lua/ai/harness/supervisor.lua`) trusts terminal outcomes as
//! given; the sink is global; `M.cancel` cancels exactly one run.
//!
//! Honest result: FAIL at `"seam"`. Every scenario's fail verdict
//! carries mechanism evidence (the recorded failure, the unreclaimed
//! hung child, the trusted false success, the verbatim hostile
//! reason). Diver-owned finding: flagged, never fixed on gauntlet
//! authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-73";
/// Human-readable name.
pub const NAME: &str = "subagent failure containment";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "failure-isolated",
    "hang-no-reclaim",
    "false-success",
    "hostile-unfiltered",
];

/// Attempt the task: failure-isolated facet first.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "failure-isolated")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"failure-isolated"`, `"hang-no-reclaim"`,
/// `"false-success"`, `"hostile-unfiltered"`. Unknown names make the
/// driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_73.lua",
        "task-73",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
