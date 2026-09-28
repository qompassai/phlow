//! task-51: byzantine worker detection (nvimlua).
//!
//! Recon probe: the design asks for byzantine worker detection on the
//! result-aggregation path — 1 of 3 workers returns plausible-but-wrong
//! results, redundancy/quorum outvotes it, an equivocating worker is
//! detected via signed/attributed results, and the dissenter is
//! identified in the evidence. The driver (`lua/gauntlet/task_51.lua`)
//! inspects the REAL fan-out consumers and verifiers — `ai.a2a.fanout`,
//! `ai.a2a.orchestrator`, `ai.harness.supervisor`, `ai.harness.verdict`
//! — reading their exported function tables (and the real result shapes
//! in source) for aggregation/quorum/voting/dissent machinery. It makes
//! no network calls and spawns no workers.
//!
//! Honest result: the seam is ABSENT. `ai.a2a.fanout.run` hands the same
//! job to N agents and calls one callback with every result in spec
//! order; `ai.a2a.orchestrator` collects per-language results with an
//! `on_partial` callback; `ai.harness.verdict.evaluate` grades ONE run's
//! acceptance criteria. No module computes a verdict over multiple
//! workers' answers, no quorum/voting rule exists, and no dissenter is
//! identified. Results ARE attributed per worker (agent / language /
//! spec index in the result shapes), so equivocation would be *visible*
//! — but nothing reads the attributed results to detect it. The
//! design's "final verdict equals the honest majority" has no seam to
//! assert against.
//!
//! Distinct from task-19: phlow-council votes on *opinions* with a safe
//! default — this design needs *fault* detection (the liar identified),
//! and there is no voting on worker results at all.
//!
//! Fail-closed: if aggregation APIs ever appear, the driver reports
//! `where = "recon"` (premise changed) instead of the seam absence.
//! Diver-owned finding: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-51";
/// Human-readable name.
pub const NAME: &str = "byzantine worker detection";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "fanout-consumers",
    "verifiers",
    "attribution-without-verdict",
    "fail-closed-recon",
];

/// Attempt the task: probe diver's fan-out consumers for result
/// aggregation machinery.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "fanout-consumers")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"fanout-consumers"`, `"verifiers"`,
/// `"attribution-without-verdict"`, `"fail-closed-recon"`. Unknown names
/// make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_51.lua",
        "task-51",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
