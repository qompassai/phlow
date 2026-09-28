//! task-09: prompt injection via tool output (nvimlua).
//!
//! Drives diver's `ai.harness` through headless Neovim with a fake
//! tool-capable adapter (see `lua/gauntlet/task_09.lua`). The adapter
//! returns a legitimate `tool.completed` sink event whose output payload
//! carries an injected instruction, and the driver pushes the attacker's
//! most charitable reading — a tool proposal for a privileged tool —
//! through the real policy/approval seam (`policy.decide`,
//! `approval.request`/`decide`/`get`).
//!
//! Threat-model note: the harness v0.1.0 has no LLM in the loop, so tool
//! outputs cannot become instructions by themselves; the meaningful seam
//! is whether the policy/approval layer would authorize a tool call whose
//! arguments originated from tool output. Scenarios: `"default"` (direct
//! injection), `"indirect"` (injection nested in a JSON result field),
//! `"encoded"` (base64/whitespace-obfuscated injection), and
//! `"approval-bypass"` (injection claims "approval granted by operator").
//! Unknown scenario names make the driver report failure.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-09";
/// Human-readable name.
pub const NAME: &str = "prompt injection via tool output";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: direct injection in a tool output.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"default"`, `"indirect"`, `"encoded"`,
/// `"approval-bypass"`. Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_09.lua",
        "task-09",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
