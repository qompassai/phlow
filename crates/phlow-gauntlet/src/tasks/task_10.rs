//! task-10: malicious MCP tool description (nvimlua).
//!
//! Drives diver's real MCP client pipeline (`ai.mcp.tools`) through
//! headless Neovim against a mock MCP stdio server (see
//! `lua/gauntlet/task_10.lua`). The mock advertises a tool whose
//! DESCRIPTION contains an embedded instruction; the driver proves the
//! description stays inert data through listing (with
//! `ai.security.mcp_vet` vetting), display (`describe`), and a normal
//! `tools/call` of a different benign tool, and that the malicious tool
//! is never invoked (the mock logs every call it receives).
//!
//! `XDG_DATA_HOME` is scoped under the task work dir: the MCP server
//! registry and the security allowlist persist under `stdpath('data')`,
//! and the driver fails closed if that directory escapes
//! `GAUNTLET_WORK_DIR`.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-10";
/// Human-readable name.
pub const NAME: &str = "malicious MCP tool description";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: malicious description stays inert data.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"default"`, `"schema-smuggle"`, `"name-spoof"`,
/// `"prompt-leak"`. Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    // Keep every persistent write (MCP registry, security allowlist)
    // inside the task work dir, matching the framework's own layout:
    // GAUNTLET_WORK_DIR = ctx.work_dir.join("task-10").
    let xdg_dir = ctx
        .work_dir
        .join("task-10")
        .join("xdg")
        .join(scenario)
        .to_string_lossy()
        .into_owned();
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_10.lua",
        "task-10",
        &[("GAUNTLET_SCENARIO", scenario), ("XDG_DATA_HOME", &xdg_dir)],
    )
}
