//! task-66: trace propagation (nvimlua).
//!
//! The design asks for a trace/correlation id that follows the run
//! across every adapter boundary crossing (ACP/A2A/MCP adapters).
//! Scenarios: default (ACP round trip — the mock agent observes the
//! same trace id the harness issued); A2A hop (the id survives
//! message/send → tasks/get); adversarial: the explicit "trace
//! continuity" check that detects a boundary dropping the id; and a
//! contract-silence check (any hop that cannot propagate must declare
//! so in its contract — no silent drops).
//!
//! Seam mapping (verified, not invented): diver's harness runs DO carry
//! an identity — `run.id` with `parent_id`/`root_id` forming a run TREE
//! (`supervisor.lua`) — but that is run provenance, not a
//! cross-boundary correlation token. The three real adapters attach
//! NOTHING trace-like to their boundary calls: ACP calls
//! `session.prompt(session_key, run.goal, cb)` (key + text only); A2A
//! calls `tasks.submit({agent, message, timeout_ms, on_done})` (no
//! envelope); MCP calls `client.start(server_spec, on_started)`
//! (server spec only). No adapter's `probe()` contract (capabilities +
//! notes) declares trace propagation OR declares the inability to
//! propagate: the drops are silent.
//!
//! The driver (`lua/gauntlet/task_66.lua`) exercises the REAL adapter
//! modules with preloaded mock peers (`package.preload`, so the real
//! adapter code paths run) that capture exactly what crosses each
//! boundary. The gauntlet plays the harness: it issues a trace id per
//! scenario and the mock peers echo what they received. The design's
//! expected result here is the documented hole: "the gap is detected
//! by an explicit 'trace continuity' check — the task fails until the
//! propagation is fixed, documenting the hole."
//!
//! Honest result: FAIL at `"seam"`. Every scenario's fail verdict
//! carries mechanism evidence (captured boundary-call shapes, the
//! failed continuity comparisons, the probe() contract reads). Diver-owned
//! finding: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-66";
/// Human-readable name.
pub const NAME: &str = "trace propagation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "acp-round-trip",
    "a2a-hop",
    "trace-continuity",
    "contract-silence",
];

/// Attempt the task: drive the real adapters with mock peers.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "acp-round-trip")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"acp-round-trip"`, `"a2a-hop"`,
/// `"trace-continuity"`, `"contract-silence"`. Unknown names make the
/// driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_66.lua",
        "task-66",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
