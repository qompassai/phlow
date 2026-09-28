//! task-06: ACP interop round-trip (nvimlua).
//!
//! Drives diver's `ai.harness` ACP adapter (`lua/ai/harness/adapters/acp.lua`)
//! end to end against a mock ACP agent (see `lua/gauntlet/task_06.lua`). The
//! mock speaks the adapter's real JSON-RPC 2.0 newline-delimited stdio
//! protocol (`initialize`, `session/new`, `session/prompt`, `session/update`
//! notifications) and is registered at runtime in `ai.acp.registry.manual`.
//! The default scenario asserts the full round-trip: streamed
//! `session/update` chunks surface as `diagnostic.observed` sink events and
//! the prompt result becomes `model.completed`, finishing the run as
//! `completed`. The named scenarios probe mid-session input
//! (`"send-input"`), the unregistered-agent rejection path
//! (`"unknown-agent"`), and transport resilience to garbage frames
//! (`"malformed-frame"`).

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-06";
/// Human-readable name.
pub const NAME: &str = "ACP interop round-trip";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: full ACP round-trip through the real adapter.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"default"`, `"send-input"`, `"unknown-agent"`,
/// `"malformed-frame"`. Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_06.lua",
        "task-06",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
