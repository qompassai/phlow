//! task-60: break-glass procedure (nvimlua).
//!
//! Recon probe: the design asks for the EMERGENCY-BYPASS path — a
//! genuine emergency needs action now with approvers unreachable, but
//! the bypass must not become a backdoor (justification required
//! post-hoc within N minutes; auto-expiry with incident flagging;
//! no standing grants). "If none exists, the design question —
//! 'should one exist?' — is the documented finding."
//!
//! The driver (`lua/gauntlet/task_60.lua`) scans the REAL ai tree —
//! the approval-path modules' export tables plus a bounded
//! source-text scan of `lua/ai` for the SPECIFIC break-glass
//! vocabulary (`break_glass`, `breakglass`, `break-glass`,
//! `emergency`). The generic `bypass`/`override` words are polysemous
//! across the tree (config overrides, cache bypasses) and are
//! classified, never counted as machinery. It makes no network calls
//! and spawns no workers.
//!
//! Honest result: no emergency-bypass path exists. The one "bypass"
//! hit is `policy.lua`'s ANTI-bypass contract ("No adapter, provider,
//! or MCP server may bypass this module") — the architecture's current
//! stance, which a break-glass path would have to be reconciled with.
//! The `ai/security` "override" hits are a control sample
//! (prompt-injection/unicode-bidi scanner vocabulary, classified
//! unrelated). The design's three required artifacts — justification
//! record, single-use time-boxed grant, auto-expiry with incident
//! flag — are all absent. The documented finding is the design's
//! question: should a break-glass procedure exist?
//!
//! Fail-closed: if bypass machinery ever appears, the driver reports
//! `where = "recon"` (premise changed) instead of the seam absence.
//! Diver-owned finding: flagged, never fixed on gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-60";
/// Human-readable name.
pub const NAME: &str = "break-glass procedure";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// The probe facets the Lua driver runs, in order:
/// two validation, two adversarial.
pub const SCENARIOS: [&str; 4] = [
    "bypass-path-scan",
    "approval-exits-closed",
    "justification-artifacts-absent",
    "fail-closed-recon",
];

/// Attempt the task: probe diver's ai tree for a break-glass path.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    run_scenario(ctx, "bypass-path-scan")
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"bypass-path-scan"`, `"approval-exits-closed"`,
/// `"justification-artifacts-absent"`, `"fail-closed-recon"`.
/// Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    crate::run_nvim_lua_driver_with_env(
        ctx,
        "task_60.lua",
        "task-60",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
