//! task-36: tool-output exfiltration (nvimlua).
//!
//! Recon probe: the design asks that a tool output containing a secret
//! (an `sk-live-...` API-key pattern) be redacted in every persisted or
//! transmitted copy — transcript, logs, report JSON — with the original
//! visible only in an explicitly-marked raw capture, and that a
//! streaming redactor catch secrets split across chunk boundaries.
//! The driver (`lua/gauntlet/task_36.lua`) exercises the REAL
//! sink/transcript path — diver's `ai.harness.events` (the append-only
//! sink: the transcript) and `ai.harness.telemetry` (the log writer) —
//! with a mock tool emitting a bare `sk-live-...` secret in its output
//! string. It makes no network calls and spawns no workers. The driver
//! never echoes the raw secret into its evidence (boolean findings
//! only), so the verdict/report JSON stays secret-free by construction.
//!
//! Honest result: the secret reaches every persisted copy VERBATIM.
//! The sink stores payloads verbatim (`events.lua`: the `redacted`
//! envelope flag is caller-set, never a scan result); `telemetry.log`
//! scrubs by KEY NAME ONLY (`telemetry.lua` `M.redact`: `REDACT_KEYS`
//! fragments matched against the key) — a secret in a string value
//! under a benign key (`output`) is never scanned for key patterns,
//! and no streaming redactor exists at all. The adjacent
//! `ai/dataaccess/secrets.redact` scrubs userinfo and assignment-style
//! secrets but not bare `sk-live-...` tokens — and it is not wired
//! into the harness sink anyway. No `sk-live` pattern exists anywhere
//! in diver's lua tree (source scan). The driver reports `fail` with
//! `where = "seam"`: the design's redaction scenario has no seam.
//! Fail-closed: if the secret ever vanishes from every persisted copy,
//! the driver reports `where = "recon"` instead. Diver-owned finding:
//! flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-36";
/// Human-readable name.
pub const NAME: &str = "tool-output exfiltration";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: probe diver's sink/transcript path for
/// secret-pattern redaction of tool outputs.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    crate::run_nvim_lua_driver(ctx, "task_36.lua", "task-36")
}
