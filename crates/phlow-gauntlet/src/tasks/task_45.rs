//! task-45: plugin dependency confusion (nvimlua).
//!
//! Recon probe: the design asks that adapter/plugin resolution be
//! explicit and trusted-first — a malicious `acp` shadowing the
//! legitimate one must never load, and an absent pinned source must
//! fail closed with "unresolved" rather than falling through to the
//! shadower. The driver (`lua/gauntlet/task_45.lua`) exercises the
//! REAL resolver: `ai.harness.registry.register_builtins` loads the
//! six built-in adapters via `pcall(require, 'ai.harness.adapters.'
//! .. name)` — a path-ordered require with no trusted-source pinning.
//!
//! Honest result: the defense the design requires is ABSENT at an
//! EXISTING seam, and the confusion is demonstrated live. The
//! adversarial scenario runs in a child nvim process (normal
//! --headless mode, where runtimepath mutation is honored) whose rtp
//! is the clean default runtimepath with the probe-owned shadow prepended
//! and the diver root appended: the
//! shadow — an `ai/harness/adapters/acp.lua` that writes an execution
//! marker into the work dir and nothing else — sits earlier on the
//! path than the legitimate diver tree. The child runs the REAL
//! `register_builtins` and reports what won: the shadow's marker IS
//! written (its code EXECUTED for the trusted name 'acp') and
//! `get_adapter('acp')` returns the shadow. The explicit registry's
//! duplicate-registration rejection does not help — the confusion
//! happens at load time, before registration — and with no pinned
//! source there is no "absent pinned source fails closed with
//! unresolved" either: any rtp entry providing the module name
//! satisfies the require. The driver reports `fail` with
//! `where = "resolution"`. Fail-closed: if `registry.lua` gains
//! pin/trust verification for built-in loading, or the shadow stops
//! winning, the driver reports `where = "recon"` instead.
//! Diver-owned security finding: flagged, never fixed on gauntlet
//! authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-45";
/// Human-readable name.
pub const NAME: &str = "plugin dependency confusion";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: probe diver's adapter resolution for
/// trusted-first ordering against a shadowed built-in.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    crate::run_nvim_lua_driver(ctx, "task_45.lua", "task-45")
}
