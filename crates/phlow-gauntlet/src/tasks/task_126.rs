//! task-126: sink-append wakes supervision, no poll acceptance probe
//! (nvim-lua, diver Phase-2 Decision 3).
//!
//! The design: the event sink gains an `on_append` subscriber hook; the
//! supervisor registers its wake callback at setup and every append invokes
//! subscribers in `pcall`; a `waking` reentrancy flag coalesces storms; no
//! periodic timer exists. `tick()` is retained as the test driver and as
//! the body `wake` invokes.
//!
//! Acceptance-probe mode: the wake surface does not exist today, so the
//! two adversarial scenarios record the exact gap (no `on_append` hook on
//! the sink, no `wake`/`waking` on the supervisor); the two validation
//! scenarios characterize today (zero repeating timers after setup — the
//! no-poll regression guard — and tick() as the sole supervision driver).
//! The gap records are the Phase-2 acceptance artifact.
//!
//! Diver probed: `c84352cc850d507df477706b9166b6541ebe9e1c` (main; no Phase-2
//! branch exists).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-126";
/// Human-readable name.
pub const NAME: &str = "sink-append-wake-no-poll";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Driver scenarios, matching `GAUNTLET_SCENARIO` in `lua/gauntlet/task_126.lua`.
pub const SCENARIOS: [&str; 4] = [
    "no-repeating-timers",
    "tick-drives-completions",
    "wake-hook-absent",
    "wake-coalescing-absent",
];

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
        "task_126.lua",
        "task-126",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
