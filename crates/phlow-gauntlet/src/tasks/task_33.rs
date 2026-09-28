//! task-33: checkpoint durability (nvimlua).
//!
//! Recon probe: the design asks for checkpoint durability across
//! SUPERVISOR death (SIGKILL) — checkpoint at step boundaries, kill the
//! supervisor, restore from the last complete checkpoint — with
//! kill-during-write atomicity (last *complete* checkpoint wins) and
//! typed rejection of schema-changed checkpoints. The driver
//! (`lua/gauntlet/task_33.lua`) exercises the REAL checkpoint module —
//! diver's `ai.harness.store` (`M.checkpoint` / `M.get_checkpoint` /
//! `M.save_run`) — with real run tables. It makes no network calls and
//! spawns no workers.
//!
//! Honest result: the checkpoint API works, but persistence is
//! IN-MEMORY ONLY (store.lua: "Phase 1 store is in-memory ... SQLite
//! backing is Phase 5 work"). A SIGKILLed supervisor takes its store
//! with it — a new supervisor process constructs a fresh `M.new()` with
//! empty tables, so `get_checkpoint` returns nil and restore is
//! impossible. Kill-during-write atomicity is vacuous (no disk writes:
//! the task scratch dir stays empty after checkpointing) and checkpoint
//! records (`{ label, at_ns, state }`) carry no schema version, so no
//! typed rejection is possible. The driver reports `fail` with
//! `where = "seam"`: the design's restore scenario has no durable seam.
//! Fail-closed: if checkpoints ever become durable, the driver reports
//! `where = "recon"` instead. Diver-owned finding: flagged, never fixed
//! on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-33";
/// Human-readable name.
pub const NAME: &str = "checkpoint durability";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: probe diver's checkpoint persistence for
/// durability across supervisor death.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    crate::run_nvim_lua_driver(ctx, "task_33.lua", "task-33")
}
