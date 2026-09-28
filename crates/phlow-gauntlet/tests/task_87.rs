//! Integration tests for task-87 (MCP session resumption).
//!
//! The seam is REAL but incapable: diver's `ai.mcp.client`
//! `teardown` deletes `sessions[name]` on process death with no
//! re-handshake, no session record, no tool-list identity, no
//! idempotency metadata, and no resumption marker. Each test drives
//! the `task_87.lua` probe in headless Neovim against the REAL
//! client and a killable scripted Python MCP stdio server, and
//! asserts the honest `fail` at `"seam"`: 2 validation, 2
//! adversarial.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`,
//! with fallbacks to Matt's known tool paths. Missing binaries or
//! directories panic with a clear message: the gauntlet fails closed,
//! never skips.

use phlow_gauntlet::tasks::task_87;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-87: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-87: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-87: HOME is not set"))
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
        "gauntlet-diver-rtp-87-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-87: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-87: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-87: cannot symlink ai: {e}"));
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
        "gauntlet-task-87-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-87: cannot build Ctx: {e}"));
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
            "task-87 passed: session resumption was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: after the server process dies between calls, the client does
/// NOT auto-resume: the session table is empty, the second handshake
/// re-negotiates from scratch (fresh wire protocolVersion), and the
/// call after it works. The task-level driver then fails at the seam:
/// resumption is manual, never automatic.
#[test]
fn no_auto_resume_after_restart() {
    assert_eq!(task_87::ID, "task-87");
    assert_eq!(task_87::NAME, "MCP session resumption");
    assert_eq!(task_87::KIND, TaskKind::NvimLua);
    assert_eq!(task_87::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_87::run_case(&ctx_for(), "restart_between_calls")
        .unwrap_or_else(|e| panic!("task-87 case failed to run: {e}"));
    assert!(
        report.passed,
        "restart-between-calls case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["auto_resumed"], false);
    assert_eq!(report.metrics["rehandshake_ok"], true);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("trace auto_resumed=Some(false)"),
        "evidence must show no automatic resumption:\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass).
    let (where_, how, _) = fail_at_seam(task_87::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-87 must fail at the seam");
    assert!(
        how.contains("no session record"),
        "the 'how' must name the missing session record: {how}"
    );
}

/// V2 (harness probe): a restarted server with a CHANGED tool list is
/// served as-is — the trace shows no invalidation API and no
/// resumption marker, so identity changes are never detected.
#[test]
fn changed_tool_list_is_not_invalidated() {
    let report = task_87::run_case(&ctx_for(), "identity_change_no_invalidation")
        .unwrap_or_else(|e| panic!("task-87 case failed to run: {e}"));
    assert!(
        report.passed,
        "identity-change case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["tools_changed"], true);
    assert_eq!(report.metrics["session_invalidated"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("trace session_invalidated=Some(false)"),
        "evidence must show the un-invalidated session:\n{joined}"
    );
}

// --- adversarial ---

/// A1: killing the server DURING a `tools/call` produces exactly one
/// wire call — no blind retry — but only a plain string error: no
/// typed `unknown_call_outcome`. The client cannot answer whether the
/// tool executed, and nothing tells the caller how to decide.
#[test]
fn inflight_death_is_untyped_and_single_attempt() {
    let report = task_87::run_case(&ctx_for(), "inflight_call_untyped")
        .unwrap_or_else(|e| panic!("task-87 case failed to run: {e}"));
    assert!(
        report.passed,
        "inflight-death case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["wire_calls"], 1);
    assert_eq!(report.metrics["typed_unknown_call_outcome"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("surfaces as a bare string"),
        "evidence must show the untyped error:\n{joined}"
    );
    assert!(
        joined.contains("trace wire_calls=Some(1)"),
        "evidence must show the single attempt:\n{joined}"
    );
}

/// A2 (harness probe): the client exposes no session-record API and
/// no resumption-boundary API — the trace asserts every known
/// introspection entry point is absent.
#[test]
fn no_session_record_api() {
    let report = task_87::run_case(&ctx_for(), "no_session_record")
        .unwrap_or_else(|e| panic!("task-87 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-session-record case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["session_record_api"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("trace session_record_api=Some(false)"),
        "evidence must show the absent API:\n{joined}"
    );
}
