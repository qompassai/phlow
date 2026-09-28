//! task-08: MCP stdio tool bridging (nvimlua).
//!
//! Drives diver's real MCP harness adapter (`ai.harness.adapters.mcp`)
//! against an MCP stdio server and proves the `tools/list` → `tools/call`
//! round trip end to end (see `lua/gauntlet/task_08.lua`). The default
//! scenario first probes the real phlow MCP server (`phlow serve`, NDJSON
//! JSON-RPC 2.0 over stdio): the probe is expected to fail the `initialize`
//! handshake because diver's `ai.mcp.client` encodes the empty capabilities
//! table as a JSON array (`"capabilities":[]`) while `phlow-mcp` strictly
//! requires a capabilities object per the MCP spec — that interop gap is
//! asserted as evidence. The round trip itself then runs against a
//! purpose-built python3 mock server (written into the work dir by the
//! driver). The adversarial scenarios steer the same mock:
//! `"bad-args"` (schema violations → typed `-32602` errors),
//! `"server-dies"` (mid-call `exit(1)` → typed process-exit error, no
//! hang), `"oversize"` (a ~10 MiB single-line response past the client's
//! 8 MiB frame cap → dropped frame, typed timeout, bounded memory).
//!
//! Environment contract: `XDG_DATA_HOME` is pointed at
//! `<work_dir>/task-08/xdg` so the security allowlist the driver writes
//! (via the real `ai.security` allowlist mechanism) stays inside the task
//! scratch directory. `GAUNTLET_PHLOW_BIN` locates the `phlow` binary,
//! falling back to the repo's debug build; a missing binary fails closed.

use crate::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Task id.
pub const ID: &str = "task-08";
/// Human-readable name.
pub const NAME: &str = "MCP stdio tool bridging";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Locate the `phlow` binary: `GAUNTLET_PHLOW_BIN` first, then the repo's
/// debug build under `$HOME`. `None` means "cannot run the real server".
fn phlow_bin() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("GAUNTLET_PHLOW_BIN") {
        let candidate = PathBuf::from(&raw);
        if !raw.is_empty() && candidate.is_file() {
            return Some(candidate);
        }
    }
    let home = std::env::var("HOME").ok()?;
    let fallback = PathBuf::from(home).join("workspace/repos/phlow/target/debug/phlow");
    fallback.is_file().then_some(fallback)
}

/// Attempt the task: the default list → call round trip.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "default")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"default"`, `"bad-args"`, `"server-dies"`,
/// `"oversize"`. Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    let xdg_dir = ctx.work_dir.join("task-08").join("xdg");
    let xdg = xdg_dir.to_string_lossy().into_owned();
    let bin = match phlow_bin() {
        Some(path) => path.to_string_lossy().into_owned(),
        None => {
            return TaskOutcome::Fail {
                where_: "environment".to_string(),
                how: "phlow serve binary not found: set GAUNTLET_PHLOW_BIN or \
                      build the phlow workspace (target/debug/phlow)"
                    .to_string(),
                evidence: vec![],
            };
        }
    };
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_08.lua",
        "task-08",
        &[
            ("GAUNTLET_SCENARIO", scenario),
            ("XDG_DATA_HOME", xdg.as_str()),
            ("GAUNTLET_PHLOW_BIN", bin.as_str()),
        ],
    )
}
