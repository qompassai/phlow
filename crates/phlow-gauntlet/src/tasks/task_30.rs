//! task-30: leader election under partition (nvimlua).
//!
//! Recon probe: the design asks for leader election among workers with
//! partition behavior, naming the harness multi-worker coordination seam
//! ("the herd adapter or supervisor may own it"). The driver
//! (`lua/gauntlet/task_30.lua`) inspects the REAL coordination modules —
//! `ai.harness.supervisor`, `ai.harness.adapters.herd`, `ai.herd`, and
//! `ai.herd.api` — for election APIs (elect/campaign/leader/heartbeat/
//! quorum/partition). It makes no network calls and spawns no workers;
//! it only reads the modules' exported function tables.
//!
//! Honest result: none of these modules implements leader election. The
//! supervisor manages run lifecycles, the herd adapter translates runs to
//! remote workers, and ai.herd manages worker agent processes —
//! coordination of *tasks*, not election of *leaders*. The driver reports
//! `fail` with `where = "seam"`: the "at most one leader" invariant has no
//! seam to assert against. Fail-closed: if election APIs ever appear, the
//! driver reports `where = "recon"` instead.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-30";
/// Human-readable name.
pub const NAME: &str = "leader election under partition";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: probe diver's coordination modules for election
/// machinery.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    crate::run_nvim_lua_driver(ctx, "task_30.lua", "task-30")
}
