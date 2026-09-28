//! task-18: worker crash recovery (nvimlua).
//!
//! Drives diver's real run supervisor (`ai.harness.supervisor`) through
//! headless Neovim (see `lua/gauntlet/task_18.lua`). A mock adapter's
//! worker "crashes"; the driver injects the crash the way a process
//! monitor would — a `diagnostic.observed` event plus
//! `supervisor.retry_run` — and proves the recovery contract:
//!
//! - transient crashes are retried with bounded attempts and backoff,
//! - the retry ceiling (`RETRY_ATTEMPTS_MAX = 4`) is exact: no fifth
//!   attempt, no silent extra retries,
//! - when the ceiling is hit the run finishes `failed` with a cause chain
//!   naming every attempt's crash, legible to the operator,
//! - duplicate crash reports for one attempt cannot spend the budget twice,
//! - the terminal finish is idempotent under a crash storm.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-18";
/// Human-readable name.
pub const NAME: &str = "worker crash recovery";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The crash-recovery scenarios the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "persistent-crash",
    "transient-crash",
    "duplicate-crash",
    "crash-storm",
];

/// Attempt the task: persistent crash → bounded retries → `failed` with
/// the cause chain.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "persistent-crash")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"persistent-crash"`, `"transient-crash"`,
/// `"duplicate-crash"`, `"crash-storm"`. Unknown names make the driver
/// report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_18.lua",
        "task-18",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
