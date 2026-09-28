//! task-115: approval render attacks (nvim-lua, adversarial).
//!
//! An operator approval UI renders proposed skill edits. This task
//! attacks the render: terminal escapes, collapsed context hiding the
//! true hunk, rationale/edit mismatch, approval fatigue (30 trivial +
//! 1 consequential), and urgency text in the rationale. The renderer
//! must strip escapes, visibly mark elisions, show per-edit behavioral
//! summaries from the OP, bound presentation rate, offer no
//! approve-all, keep urgency out of chrome, and bind approvals to
//! SHA-256 of exact bytes.
//!
//! (`lua/gauntlet/task_115.lua`: self-contained approval renderer +
//! attack fixtures. No diver seam is used; the render is task-local,
//! stated honestly.)

use crate::TaskKind;
use crate::TaskOutcome;
use crate::tasks::Ctx;

/// Task id.
pub const ID: &str = "task-115";
/// Task name.
pub const NAME: &str = "approval render attacks";
/// Task kind.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_115.lua`.
pub const SCENARIOS: &[&str] = &[
    "clean", "escapes", "elision", "urgency", "mismatch", "fatigue",
];

/// Run all scenarios; any failure is a breach.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    let mut evidence = Vec::new();
    for scenario in SCENARIOS {
        match run_scenario(ctx, scenario) {
            TaskOutcome::Pass { evidence: e } => evidence.extend(e),
            TaskOutcome::Fail {
                where_,
                how,
                evidence: e,
            } => {
                evidence.extend(e);
                return TaskOutcome::Fail {
                    where_: format!("task-115/{scenario}/{where_}"),
                    how,
                    evidence,
                };
            }
        }
    }
    TaskOutcome::Pass { evidence }
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_115.lua",
        "task-115",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
