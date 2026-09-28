//! task-57: escalation chains (nvimlua).
//!
//! Recon probe: the design asks for an approval ROUTING table
//! (L1 → L2 → …) — first approver unavailable, request routes to L2
//! with full context; chain order as configuration; exhaustion → deny.
//! The driver (`lua/gauntlet/task_57.lua`) inspects the REAL approval
//! surfaces — `ai.harness.approval`, `ai.harness.supervisor`,
//! `ai.harness` (the public setup/run/cancel/resume API) — reading
//! exported function tables for routing vocabulary, plus a bounded
//! source-text scan of the harness tree. It makes no network calls and
//! spawns no workers.
//!
//! Honest result: the seam is ABSENT. The approval module is a FLAT
//! queue (request/decide/get/pending/sweep_expired): one request, one
//! decision, no levels, no next-approver, no delegation. The harness
//! public API takes no approver-chain configuration. The design's three
//! required artifacts — routing table, chain order as configuration,
//! exhaustion → deny rule — are all absent. Known-unrelated vocabulary
//! (store.lua's "fallback" hash label, adapter.lua's "delegates"
//! comment) is classified, never counted.
//!
//! Fail-closed: if routing machinery ever appears on the approval path,
//! the driver reports `where = "recon"` (premise changed) instead of
//! the seam absence.
//! Diver-owned finding: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-57";
/// Human-readable name.
pub const NAME: &str = "escalation chains";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "routing-surface",
    "chain-order-config",
    "exhaustion-undefined",
    "fail-closed-recon",
];

/// Attempt the task: probe diver's approval path for routing machinery.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "routing-surface")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"routing-surface"`, `"chain-order-config"`,
/// `"exhaustion-undefined"`, `"fail-closed-recon"`. Unknown names make
/// the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_57.lua",
        "task-57",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
