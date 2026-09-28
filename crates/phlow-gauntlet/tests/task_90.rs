//! Integration tests for task-90 (namespaced cross-protocol dispatch).
//!
//! The seam is ABSENT as designed: diver's harness has NO namespaced
//! cross-protocol dispatcher — routing is per-run adapter binding
//! (`spec.adapter`), calls stay native to their adapter, and the
//! tool registry rejects names outside `^[a-z][a-z0-9_]*$`, so
//! `proto:name` addressing is inexpressible. Each test drives the
//! `task_90.lua` probe in headless Neovim against the REAL
//! `ai.harness` modules and asserts the honest `fail` at `"seam"`:
//! 2 validation, 2 adversarial.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`,
//! with fallbacks to Matt's known tool paths. Missing binaries or
//! directories panic with a clear message: the gauntlet fails closed,
//! never skips.

use phlow_gauntlet::tasks::task_90;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-90: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-90: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-90: HOME is not set"))
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
        "gauntlet-diver-rtp-90-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-90: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-90: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-90: cannot symlink ai: {e}"));
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
        "gauntlet-task-90-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-90: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(180);
    ctx
}

/// Unwrap the expected `fail` at `"seam"`, or panic with the details.
fn fail_at_seam(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-90 passed: a namespaced dispatcher was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: the real `mcp` and `a2a` adapter modules are distinct
/// registrations and an mcp run against the mock completes with only
/// mcp/supervisor-sourced events — adapters are disjoint and
/// protocol-pure by construction.
#[test]
fn adapters_are_disjoint() {
    assert_eq!(task_90::ID, "task-90");
    assert_eq!(task_90::NAME, "namespaced cross-protocol dispatch");
    assert_eq!(task_90::KIND, TaskKind::NvimLua);
    assert_eq!(task_90::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_90::run_case(&ctx_for(), "adapters_disjoint")
        .unwrap_or_else(|e| panic!("task-90 case failed to run: {e}"));
    assert!(
        report.passed,
        "adapters-disjoint case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["adapters_disjoint"], true);
    assert_eq!(report.metrics["cross_protocol_traffic"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("adapters_disjoint=Some(true)"),
        "evidence must show the disjoint adapters:\n{joined}"
    );
}

/// V2: there is no namespace entry point — `ai.harness` exposes only
/// setup/run/cancel/resume/version, the registry only
/// adapter/tool/workflow registration, and `register_tool` REJECTS
/// `mcp:summarize` on the name pattern. The task-level driver then
/// fails at the seam: the namespaced dispatcher is absent as
/// designed.
#[test]
fn no_namespace_entry_point() {
    let report = task_90::run_case(&ctx_for(), "no_namespace_entry")
        .unwrap_or_else(|e| panic!("task-90 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-namespace case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["dispatch_entry"], false);
    assert_eq!(report.metrics["colon_names_rejected"], true);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("colon_names_rejected=Some(true)"),
        "evidence must show the rejected namespaced name:\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass).
    let (where_, how, _) = fail_at_seam(task_90::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-90 must fail at the seam");
    assert!(
        how.contains("no namespaced cross-protocol dispatcher"),
        "the 'how' must name the absent dispatcher: {how}"
    );
}

// --- adversarial ---

/// A1 (harness probe): a mock tool literally named `a2a:send` is
/// called with the literal name on the MCP wire — the trace shows
/// `prefix_parsed=false` and `cross_protocol_dispatch=false`. The
/// spoofed prefix is treated opaquely: safe only because no namespace
/// machinery exists at all.
#[test]
fn spoofed_prefix_stays_opaque() {
    let report = task_90::run_case(&ctx_for(), "spoof_prefix_opaque")
        .unwrap_or_else(|e| panic!("task-90 case failed to run: {e}"));
    assert!(
        report.passed,
        "spoof case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["cross_protocol_dispatch"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("cross_protocol_dispatch=Some(false)"),
        "evidence must show no cross-protocol dispatch:\n{joined}"
    );
}

/// A2 (harness probe): a bare-name duplicate is rejected, but a
/// cross-protocol collision is inexpressible rather than rejected as
/// ambiguous — the trace shows `bare_duplicate_rejected=true` and no
/// namespaced lookup. The design's ambiguous-name rejection is
/// vacuous because unqualified names are the only names that exist.
#[test]
fn ambiguous_names_are_inexpressible() {
    let report = task_90::run_case(&ctx_for(), "ambiguous_inexpressible")
        .unwrap_or_else(|e| panic!("task-90 case failed to run: {e}"));
    assert!(
        report.passed,
        "ambiguous case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["bare_duplicate_rejected"], true);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("bare_duplicate_rejected=Some(true)"),
        "evidence must show the duplicate rejection:\n{joined}"
    );
}
