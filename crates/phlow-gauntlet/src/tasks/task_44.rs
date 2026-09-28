//! task-44: egress filtering (nvimlua).
//!
//! Recon probe: the design asks that every request's *resolved*
//! destination be checked against the allowlist at the tool/adapter
//! network path (MCP stdio excluded) — public HTTPS allowed,
//! `http://169.254.169.254/` blocked, DNS-rebinding (`evil.com`
//! resolving to 127.0.0.1) blocked on the resolved IP rather than the
//! hostname, redirect chains re-checked per hop, blocks explicit and
//! logged. The driver (`lua/gauntlet/task_44.lua`) probes diver's real
//! network surface — the adapter registry and `ai.rose.http` — with a
//! mock fetcher carrying controllable DNS/redirects.
//!
//! Honest result: the filtering seam is ABSENT. Diver's ai.harness
//! has no URL-fetch tool — the registry holds no fetch/http/url tool —
//! so the design's "tool requests an internal address" scenarios have
//! nothing to attach to. The adjacent `ai.rose.http` is an
//! operator-configured model-endpoint client, not an agent URL-fetch
//! tool: its URL check is hostname-string based with no DNS
//! resolution and no resolved-IP filtering; its curl path refuses
//! redirects outright (`--max-redirs 0`) rather than re-checking per
//! hop. The driver reports `fail` with `where = "seam"`.
//! Fail-closed: if diver gains a URL-fetch tool or a
//! resolved-destination filter, the driver reports
//! `where = "recon"` instead.
//! Diver-owned finding: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-44";
/// Human-readable name.
pub const NAME: &str = "egress filtering";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Attempt the task: probe diver's tool/adapter network path for
/// resolved-destination egress filtering.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    crate::run_nvim_lua_driver(ctx, "task_44.lua", "task-44")
}
