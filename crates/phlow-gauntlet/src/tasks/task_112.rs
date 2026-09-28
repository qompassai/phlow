//! task-112: skill-document prompt injection (nvim-lua, adversarial).
//!
//! The SkillOpt optimizer consumes a skill document that may contain
//! hostile text. This task drives the three design poison fixtures plus
//! two adversarial probes through diver's REAL harness include path
//! (`lua/ai/harness/context.lua`: trust-classified immutable snapshots +
//! seal) under headless Neovim, with a task-local fence-and-quote prompt
//! assembly (diver has no native optimizer-prompt render seam — the
//! harness renders metadata manifests only; the data/instruction boundary
//! is supplied by this task, stated honestly).
//!
//! A mock optimizer stands in for a model that follows `OPTIMIZER:`
//! directives wherever it sees them unless they are fenced as data. The
//! bar: mock proposals must match the clean baseline for every poison
//! fixture (byte-identical); instruction-like log content must remain
//! quoted; unknown directives must stay inert. Any deviation is a breach
//! naming the injection and the failed assembly point.
//!
//! Diver probed: `60970812466e7ae54ce851b876ea4240c52aa325` (main).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-112";
/// Human-readable name.
pub const NAME: &str = "skill-document prompt injection";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_112.lua`.
/// 2 validation + 2 adversarial.
pub const SCENARIOS: [&str; 4] = ["clean", "poison", "logquote", "unknown"];

/// Attempt the task: run every driver scenario and report the first
/// failure, so the recorded outcome names the exact gap. All four must
/// pass for the task to pass.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    let mut evidence = Vec::new();
    for scenario in SCENARIOS {
        match run_scenario(ctx, scenario) {
            TaskOutcome::Pass { evidence: ev } => evidence.extend(ev),
            TaskOutcome::Fail {
                where_,
                how,
                evidence: ev,
            } => {
                evidence.extend(ev);
                return TaskOutcome::Fail {
                    where_,
                    how: format!("scenario {scenario:?}: {how}"),
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
        "task_112.lua",
        "task-112",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
