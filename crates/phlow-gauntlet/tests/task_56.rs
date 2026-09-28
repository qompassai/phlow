//! Integration tests for task-56 (approval timeout defaults deny).
//!
//! The seam is PRESENT: diver's `ai.harness.approval` expires overdue
//! requests via `supervisor.tick` → `sweep_expired`, and only pending
//! approvals can be decided. Each test drives `task_56.lua` in headless
//! Neovim against the REAL diver Lua tree and asserts the `pass` verdict:
//! 2 validation, 2 adversarial.
//!
//! Honest naming note: the design says "denied" and the module header
//! says "Requests expire to denied", but the code's terminal state is
//! named `expired` — the deny-equivalent (terminal, never approved,
//! decide() rejects it). The tests assert the security property
//! (default-deny on timeout), not the label.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_56;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-56: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-56: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-56: HOME is not set"))
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
        "gauntlet-diver-rtp-56-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-56: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-56: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-56: cannot symlink ai: {e}"));
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
        "gauntlet-task-56-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-56: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(180);
    ctx
}

/// Unwrap the expected `pass` verdict, or panic with the details.
fn pass_verdict(outcome: TaskOutcome) -> Vec<String> {
    match outcome {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-56 failed: where={where_} how={how}\nevidence: {evidence:?}"),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the control scenario (approver
/// responds in time) passes — a timely approval grants and the gated
/// tool proceeds.
#[test]
fn timely_approval_grants_and_tool_proceeds() {
    assert_eq!(task_56::ID, "task-56");
    assert_eq!(task_56::NAME, "approval timeout defaults deny");
    assert_eq!(task_56::KIND, TaskKind::NvimLua);
    assert_eq!(task_56::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let evidence = pass_verdict(task_56::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("scenario=responds-in-time"),
        "evidence must name the scenario:\n{joined}"
    );
    assert!(
        joined.contains("stand-in approver decided approved before deadline"),
        "evidence must show the timely decision:\n{joined}"
    );
}

/// V: with nobody responding, the request reaches a terminal state by
/// deadline + epsilon — pinned at the boundary: pending at
/// deadline − 1ns, terminal at deadline. Never stuck pending.
#[test]
fn silent_request_is_terminal_by_deadline_plus_epsilon() {
    let evidence = pass_verdict(task_56::run_scenario(
        &ctx_for(),
        "terminal-within-deadline",
    ));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("tick(deadline - 1ns): state=pending"),
        "evidence must pin the pre-deadline boundary:\n{joined}"
    );
    assert!(
        joined.contains("tick(deadline): state="),
        "evidence must show the at-deadline state:\n{joined}"
    );
    assert!(
        !joined.contains("tick(deadline): state=pending"),
        "the request must not still be pending at the deadline:\n{joined}"
    );
}

// --- adversarial ---

/// A: the deadline passes with no response — the request is NOT granted
/// (no default-allow) and NOT left pending forever; the gate blocks the
/// tool, proved positively by the absent marker file.
#[test]
fn timeout_defaults_to_deny_never_grants() {
    let evidence = pass_verdict(task_56::run_scenario(&ctx_for(), "timeout-never-grants"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("mock approver assigned but silent"),
        "evidence must name the silent-approver dimension:\n{joined}"
    );
    assert!(
        joined.contains("positive proof the tool never ran"),
        "evidence must prove non-execution positively:\n{joined}"
    );
    assert!(
        !joined.contains("TIMEOUT GRANTED"),
        "a timeout grant would be the default-allow bug:\n{joined}"
    );
}

/// A: the approver's response arrives after the expiry — the late
/// 'approved' is rejected with an explicit already-decided error and the
/// record stays expired: a late approval cannot resurrect the request.
#[test]
fn late_approval_cannot_resurrect() {
    let evidence = pass_verdict(task_56::run_scenario(
        &ctx_for(),
        "late-approval-cannot-resurrect",
    ));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("late decide(approved): ok=false"),
        "evidence must show the late decide was rejected:\n{joined}"
    );
    assert!(
        joined.contains("already"),
        "the rejection must be an explicit already-decided rejection:\n{joined}"
    );
    assert!(
        !joined.contains("RESURRECTED"),
        "a resurrected request would be the failure mode:\n{joined}"
    );
}
