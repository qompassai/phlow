//! task-69: hallucinated tool rejection (nvimlua).
//!
//! The design asks for the tool dispatcher (name → implementation):
//! the model invents a tool that doesn't exist. Scenarios: default
//! (real tool name → dispatched); hallucination (`read_files_fast`
//! when only `read_file` exists → clean `unknown_tool` rejection —
//! *no* fuzzy matching, *no* did-you-mean execution); adversarial:
//! near-miss names at edit-distance 1 of a privileged tool (still
//! rejected — similarity is not authority); adversarial: hallucinated
//! tool *with* valid args (rejected on the name, before args are even
//! parsed). Pass criteria: the dispatcher is exact-match only (proven
//! by the near-miss battery); the rejection suggests nothing
//! executable. Distinct from task-04 (unknown *adapter* at config
//! time) — this is an unknown *tool* at model-output time; and from
//! task-10 (the tool exists but its description is malicious).
//!
//! Seam mapping (verified, not invented): diver's real dispatcher is
//! `ai.rose.tools.M.call(name, args)` — "Call a tool by name; never
//! raises, always returns a status table." Dispatch is a table lookup
//! `by_name[name]` with `assert(spec, 'unknown tool: ' .. tostring(name))`
//! BEFORE `validate_args(args, spec)`: name resolution precedes arg
//! parsing by construction. The module contains no fuzzy matching, no
//! edit-distance, no did-you-mean (verified by source read).
//!
//! The driver (`lua/gauntlet/task_69.lua`) exercises the REAL
//! dispatcher with scripted caller names: real-tool dispatch,
//! the design's hallucination example, a 10-name edit-distance-1
//! battery (including the privileged `file_write`), and the
//! name-before-args ordering proof.
//!
//! Honest result: PASS.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-69";
/// Human-readable name.
pub const NAME: &str = "hallucinated tool rejection";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "real-tool-dispatches",
    "hallucination",
    "near-miss-battery",
    "name-before-args",
];

/// Attempt the task: drive the real `ai.rose.tools.M.call`.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "real-tool-dispatches")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"real-tool-dispatches"`, `"hallucination"`,
/// `"near-miss-battery"`, `"name-before-args"`. Unknown names make the
/// driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_69.lua",
        "task-69",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
