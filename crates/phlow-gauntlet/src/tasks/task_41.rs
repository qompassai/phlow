//! task-41: authority attenuation (nvimlua).
//!
//! Recon probe: the design asks that the effective tool set of any run
//! be the intersection of its ancestors' grants — a parent run with
//! tools {read} delegating to a child must deny the child tool {write}
//! (child authority ⊆ parent authority), transitively at depth 2, and
//! the denial names the missing grant. The driver
//! (`lua/gauntlet/task_41.lua`) exercises the REAL delegation path —
//! diver's `ai.harness.supervisor` (`M.create` / `M.spawn_child` /
//! `M.parent_id`) — with mock tools at distinct privilege levels.
//!
//! Honest result: the attenuation seam is ABSENT. Diver has real
//! delegation — `spawn_child` sets `parent_id` and calls `create` — but
//! run tables carry no tool grant, no authority set, and no privilege
//! list, and `spawn_child` performs no grant-intersection step.
//! Harness policy is supervisor-global, not inherited per run. The
//! design's "child authority ⊆ parent authority", transitive
//! attenuation, and named-grant denial have no seam to attach to, so
//! all four cases (default {read} use, adversarial {write} request,
//! adversarial grandchild {write}, and the structural seam check)
//! report the absence. The driver reports `fail` with
//! `where = "seam"`. Fail-closed: if run tables gain a tool grant and
//! `spawn_child` intersects it (or the delegation path otherwise
//! changes), the driver reports `where = "recon"` instead.
//! Diver-owned finding: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-41";
/// Human-readable name.
pub const NAME: &str = "authority attenuation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: probe diver's delegation path for per-run
/// authority attenuation.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    crate::run_nvim_lua_driver(ctx, "task_41.lua", "task-41")
}
