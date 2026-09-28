//! Integration tests for task-57 (escalation chains).
//!
//! The seam is ABSENT: diver has no approval routing table —
//! `ai.harness.approval` is a flat queue (request/decide/get/pending/
//! sweep_expired), the harness public API takes no approver-chain
//! configuration, and no exhaustion → deny rule exists for a chain
//! that does not exist. Each test drives the `task_57.lua` recon probe
//! in headless Neovim against the REAL diver Lua tree and asserts
//! aspects of the honest `fail` verdict (`where = "seam"`):
//! 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_57;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-57: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-57: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-57: HOME is not set"))
}

/// Process-local sequence so concurrent `ctx_for` calls never collide.
static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build a scratch diver runtime-path shim: the Lua drivers append
/// `DIVER_LUA_DIR` to the rtp and `require('ai....')`, which needs
/// `<dir>/lua/ai/...` on the rtp. The shim is two symlinks —
/// `lua -> <diver-lua>` and `ai -> <diver-lua>/ai` — in a scratch dir;
/// no diver file is touched. The real tree comes from
/// `GAUNTLET_DIVER_LUA` (or Matt's known checkout).
fn diver_rtp_shim(seq: u64) -> PathBuf {
    let real = required_path(
        "GAUNTLET_DIVER_LUA",
        &format!("{}/workspace/repos/diver/lua", home_dir()),
    );
    let shim = std::env::temp_dir().join(format!(
        "gauntlet-diver-rtp-57-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-57: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-57: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-57: cannot symlink ai: {e}"));
    }
    shim
}

/// Build a `Ctx` with its own scratch directory and diver rtp shim.
/// The workdir is unique per call (pid + a process-local counter):
/// tests running in parallel get disjoint directories.
fn ctx_for() -> Ctx {
    let nvim_bin = required_path(
        "GAUNTLET_NVIM_BIN",
        &format!("{}/workspace/tools/neovim-nightly/bin/nvim", home_dir()),
    );
    let seq = WORKDIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let diver_lua = diver_rtp_shim(seq);
    let work_dir = std::env::temp_dir().join(format!(
        "gauntlet-task-57-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-57: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(180);
    ctx
}

/// Unwrap the expected `fail` verdict, or panic with the details.
fn fail_verdict(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-57 passed: routing machinery was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the routing-surface probe
/// completes and reports the seam absence — `where = "seam"`, naming
/// the missing routing table. The approval module's real export table
/// (request/decide/get/pending/sweep_expired) must appear in evidence:
/// a flat queue, not a chain.
#[test]
fn probe_reports_routing_absence() {
    assert_eq!(task_57::ID, "task-57");
    assert_eq!(task_57::NAME, "escalation chains");
    assert_eq!(task_57::KIND, TaskKind::NvimLua);
    assert_eq!(task_57::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_verdict(task_57::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-57 must fail at the absent seam");
    let joined_ev = evidence.join("\n");
    assert!(
        joined_ev.contains("ai.harness.approval"),
        "the approval module must actually load:\n{joined_ev}"
    );
    assert!(
        joined_ev.contains("sweep_expired"),
        "evidence must show the flat-queue export table:\n{joined_ev}"
    );
    assert!(
        !joined_ev.contains("require failed"),
        "no module require may fail silently:\n{joined_ev}"
    );
    assert!(
        how.contains("no approval routing table"),
        "the 'how' must name the absent routing table: {how}"
    );
}

/// V: the chain-order facet — the harness public API
/// (setup/run/cancel/resume/version) takes no approver-chain
/// configuration, so "chain order is configuration, not code" has no
/// configuration surface to assert against.
#[test]
fn chain_order_has_no_configuration_surface() {
    let (where_, _how, evidence) =
        fail_verdict(task_57::run_scenario(&ctx_for(), "chain-order-config"));
    assert_eq!(
        where_, "seam",
        "a missing chain must not flip the verdict to 'recon'"
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("setup/run/cancel/resume/version"),
        "evidence must show the public API surface:\n{joined}"
    );
    assert!(
        joined.contains("no approver-chain option"),
        "evidence must state the missing chain option:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_57::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: the design's escalation scenarios are documented vacuous — no L1/L2
/// to route between, no chain to exhaust, no late-escalation
/// double-decide to guard — and the fail-closed facet records zero
/// routing hits explicitly, classifying the known-unrelated vocabulary.
#[test]
fn escalation_scenarios_documented_vacuous() {
    let (_where_, _how, evidence) =
        fail_verdict(task_57::run_scenario(&ctx_for(), "exhaustion-undefined"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no L1, no L2, no route"),
        "evidence must document the vacuous routing scenario:\n{joined}"
    );
    assert!(
        joined.contains("no hops exist"),
        "evidence must document the missing hop log:\n{joined}"
    );
    let (_where2, _how2, evidence2) =
        fail_verdict(task_57::run_scenario(&ctx_for(), "fail-closed-recon"));
    let joined2 = evidence2.join("\n");
    assert!(
        joined2.contains("0 routing hits"),
        "fail-closed facet must record zero hits explicitly:\n{joined2}"
    );
    assert!(
        joined2.contains("UNRELATED"),
        "fail-closed facet must classify the known-unrelated vocabulary:\n{joined2}"
    );
}
