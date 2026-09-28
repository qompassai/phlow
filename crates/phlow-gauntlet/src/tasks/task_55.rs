//! task-55: dissent escalation (nvimlua).
//!
//! Recon probe: the design asks for dissent escalation on the
//! multi-model answer comparison path — two heterogeneous models answer
//! the same prompt; on agreement the run proceeds, on disagreement the
//! conflict is escalated to a human with both answers quoted (never a
//! silent majority), and a systematically-wrong model trips a
//! dissent-rate alert and is quarantined from the comparison set pending
//! review, the quarantine threshold a named constant. The driver
//! (`lua/gauntlet/task_55.lua`) inspects the REAL comparison surfaces —
//! the six harness adapters (rose, phlow, a2a, herd, mcp, acp: the
//! modules that return answers to prompts), `ai.harness.verdict`, and
//! `ai.security` (the one module with escalation/quarantine vocabulary,
//! the control sample) — reading exported function tables for
//! answer-comparison / dissent / escalation machinery. It makes no
//! network calls and spawns no workers.
//!
//! Honest result: the seam is ABSENT. Adapters start workers and return
//! handles; nothing feeds two models' answers into a comparator;
//! `ai.harness.verdict.evaluate` grades ONE run's acceptance criteria;
//! escalation/quarantine in `ai.security` concern suspicious FILES
//! (composite security verdicts; quarantine moves a file to an isolated
//! directory), not model answers. There is no escalation record, no
//! dissent-rate counter, and no quarantine-threshold constant for
//! models. The design's "no action on a dissented answer without human
//! review" holds vacuously — no dissent is ever detected — which is
//! exactly the failure mode the design forbids: silent non-detection,
//! not silent majority.
//!
//! Distinct from task-19 (the council VOTES and a tie resolves to the
//! safe default Revise): this design refuses to resolve machine-side at
//! all — and there is no machine-side path to refuse with.
//!
//! Fail-closed: if comparison machinery ever appears, the driver reports
//! `where = "recon"` (premise changed) instead of the seam absence.
//! Diver-owned finding: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-55";
/// Human-readable name.
pub const NAME: &str = "dissent escalation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "adapters",
    "verdict-and-security",
    "no-escalation-record",
    "fail-closed-recon",
];

/// Attempt the task: probe diver's answer path for comparison
/// machinery.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "adapters")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"adapters"`, `"verdict-and-security"`,
/// `"no-escalation-record"`, `"fail-closed-recon"`. Unknown names make
/// the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_55.lua",
        "task-55",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
