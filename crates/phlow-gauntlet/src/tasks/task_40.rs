//! task-40: approval TOCTOU (nvimlua).
//!
//! Recon probe: the design asks that EVERY execution re-validate the
//! approval against CURRENT state (or that the approval bind a state
//! hash), with the check after the last possible mutation point. The
//! driver (`lua/gauntlet/task_40.lua`) exercises the REAL approval
//! module — diver's `ai.harness.approval` (`M.new` / `M.request` /
//! `M.decide` / `M.get` / `M.pending` / `M.sweep_expired`) — with a mock
//! approver and a mutator that races execution (symlink swap of the
//! approved target; grant-then-revoke). It makes no network calls and
//! spawns no workers.
//!
//! Honest result: the re-validation seam is ABSENT, twice over.
//! Approval records bind no approved-against state — the record is
//! `{ id, run_id, tool, risk, summary, argv?, paths?, endpoints?,
//! state, created_ns, deadline_ns, decided_by? }` with no state hash,
//! digest, or fingerprint. And no execution-time re-validation exists:
//! nothing in the harness consumes an approval at execution (the
//! supervisor only sweeps expiries in `tick()`); there is no executor
//! function that re-checks an approval against current state, no
//! revocation API (`M.decide` accepts only 'approved'/'denied' from
//! 'pending' — an approved record can never move back), and no liveness
//! check. The design's "executor re-validates the target" and
//! "execution checks liveness" have no seam to attach to. The driver
//! reports `fail` with `where = "seam"`. Fail-closed: if the record
//! gains a state-hash field, a revocation path, or an execution-time
//! re-validation call, the driver reports `where = "recon"` instead.
//! Diver-owned finding: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-40";
/// Human-readable name.
pub const NAME: &str = "approval TOCTOU";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: probe diver's approval record → execution path
/// for re-validation against current state.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    crate::run_nvim_lua_driver(ctx, "task_40.lua", "task-40")
}
