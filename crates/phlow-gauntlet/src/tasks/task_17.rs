//! task-17: nvim edit-check-fix loop (nvimlua).
//!
//! Drives `lua/gauntlet/task_17.lua` through headless Neovim. The driver
//! simulates a harness-driven agent's loop: write a Lua file carrying a
//! syntax error, run `luac -p` on it through job control (`vim.system`),
//! parse the error out of stderr, apply a fix, and re-verify — passing
//! only when `luac` exits 0.
//!
//! Scenarios (via `GAUNTLET_SCENARIO`): `"default"` (one error, one fix,
//! clean), `"bad-fix"` (the first fix is itself broken with a *different*
//! error; the loop must catch the new error and keep going), and
//! `"no-converge"` (fixes oscillate between two errors; the loop must
//! stop after a bounded number of attempts and fail honestly).
//!
//! The `luac` binary resolves from `GAUNTLET_LUAC_BIN`, else a PATH search.
//! A missing `luac` is a fail-closed driver error, never a silent skip.

use crate::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Task id.
pub const ID: &str = "task-17";
/// Human-readable name.
pub const NAME: &str = "nvim edit-check-fix loop";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: the default edit -> check -> fix -> verify loop.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"default"`, `"bad-fix"`, `"no-converge"`. Unknown
/// names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    match resolve_luac_bin() {
        Ok(bin) => run_scenario_with_luac(ctx, scenario, &bin),
        Err(how) => TaskOutcome::Fail {
            where_: "check".to_string(),
            how,
            evidence: vec![],
        },
    }
}

/// [`run_scenario`] with an explicit `luac` binary, forwarded to the
/// driver as `GAUNTLET_LUAC_BIN`. The binary is passed by path rather
/// than mutated process environment so concurrent callers cannot race.
pub fn run_scenario_with_luac(
    ctx: &Ctx,
    scenario: &str,
    luac_bin: &std::path::Path,
) -> TaskOutcome {
    let luac_str = luac_bin.to_string_lossy();
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_17.lua",
        "task-17",
        &[
            ("GAUNTLET_SCENARIO", scenario),
            ("GAUNTLET_LUAC_BIN", luac_str.as_ref()),
        ],
    )
}

/// Resolve the `luac` binary: `GAUNTLET_LUAC_BIN` wins, then a search of
/// `PATH` for an executable file named `luac`. A missing binary is an
/// explicit error — the task fails closed rather than skipping the check.
fn resolve_luac_bin() -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var("GAUNTLET_LUAC_BIN")
        && !raw.is_empty()
    {
        return Ok(PathBuf::from(raw));
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("luac");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("luac not found: set GAUNTLET_LUAC_BIN to the luac binary or put luac on PATH".to_string())
}
