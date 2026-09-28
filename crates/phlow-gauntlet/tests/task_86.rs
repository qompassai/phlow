//! Integration tests for task-86 (MCP capability negotiation mismatch).
//!
//! The seam is REAL but incapable: diver's `ai.mcp.client`
//! sends `capabilities = {}` in `initialize` and discards the
//! server's advertised capabilities — it never records or gates on
//! negotiated capabilities, so any method is sent on any ready
//! session. Each test drives the `task_86.lua` probe in headless
//! Neovim against the REAL client and a scripted Python MCP stdio
//! server, and asserts the honest `fail` at `"seam"`: 2 validation,
//! 2 adversarial.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`,
//! with fallbacks to Matt's known tool paths. Missing binaries or
//! directories panic with a clear message: the gauntlet fails closed,
//! never skips.

use phlow_gauntlet::tasks::task_86;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-86: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-86: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-86: HOME is not set"))
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
        "gauntlet-diver-rtp-86-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-86: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-86: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-86: cannot symlink ai: {e}"));
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
        "gauntlet-task-86-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-86: cannot build Ctx: {e}"));
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
            "task-86 passed: capability gating was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: matching capabilities round-trip cleanly against the REAL
/// client — handshake_ok, tool-list contains `ping`, `tools/call`
/// succeeds — but the client records nothing about the negotiated
/// capabilities: `has_capability_fn` is false and the wire carried
/// the Lua client's fixed empty capabilities. The task-level driver
/// then fails at the seam: there is nothing to gate on.
#[test]
fn matching_roundtrip_works_but_nothing_is_recorded() {
    assert_eq!(task_86::ID, "task-86");
    assert_eq!(task_86::NAME, "MCP capability negotiation mismatch");
    assert_eq!(task_86::KIND, TaskKind::NvimLua);
    assert_eq!(task_86::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_86::run_case(&ctx_for(), "matching_roundtrip")
        .unwrap_or_else(|e| panic!("task-86 case failed to run: {e}"));
    assert!(
        report.passed,
        "matching-roundtrip case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["handshake_ok"], true);
    assert_eq!(report.metrics["roundtrip_ok"], true);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("server_advertised=Some([String(\"tools\")])"),
        "evidence must show the advertised tools:\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass).
    let (where_, how, _) = fail_at_seam(task_86::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-86 must fail at the seam");
    assert!(
        how.contains("gates nothing on negotiated capabilities"),
        "the 'how' must name the missing gating: {how}"
    );
}

/// V2 (harness probe): over the driver's machine-readable trace,
/// there is no capability/session introspection API and the wire
/// shows the client's fixed empty capabilities — the negotiation is
/// never recorded.
#[test]
fn negotiation_is_never_recorded() {
    let report = task_86::run_case(&ctx_for(), "negotiation_unrecorded")
        .unwrap_or_else(|e| panic!("task-86 case failed to run: {e}"));
    assert!(
        report.passed,
        "negotiation-unrecorded case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["client_advertised_caps"], "[]");
    assert_eq!(report.metrics["negotiated_record"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("negotiated_record=Some(false)"),
        "evidence must show the absent negotiation record:\n{joined}"
    );
}

// --- adversarial ---

/// A1: a server that ADVERTISES `tools` but then fails every
/// `tools/list` yields only a plain string error — no typed
/// `capability_mismatch`. The lie is never checked against the
/// advertisement.
#[test]
fn advertising_lie_has_no_typed_mismatch() {
    let report = task_86::run_case(&ctx_for(), "tools_lie_untyped")
        .unwrap_or_else(|e| panic!("task-86 case failed to run: {e}"));
    assert!(
        report.passed,
        "tools-lie case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["typed_capability_mismatch"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no typed `capability_mismatch` names"),
        "evidence must show the untyped error:\n{joined}"
    );
}

/// A2 (harness probe): the server advertises only `tools`, but the
/// client's `resources/list` call still goes onto the wire — the
/// trace shows the method was sent despite never being advertised.
#[test]
fn unadvertised_method_still_goes_on_the_wire() {
    let report = task_86::run_case(&ctx_for(), "unadvertised_call_sent")
        .unwrap_or_else(|e| panic!("task-86 case failed to run: {e}"));
    assert!(
        report.passed,
        "unadvertised-call case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["unadvertised_call_sent"], true);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("trace unadvertised_call_sent=Some(true)"),
        "evidence must show the ungated send:\n{joined}"
    );
}
