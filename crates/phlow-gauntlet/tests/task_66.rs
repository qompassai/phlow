//! Integration tests for task-66 (trace propagation).
//!
//! The seam is ABSENT: diver has run identity (`run.id` / `parent_id` /
//! `root_id` — a run tree) but NO trace/correlation id that crosses
//! adapter boundaries. Each test drives the `task_66.lua` probe in
//! headless Neovim against the REAL diver adapter modules (the peers
//! are mocks via `package.preload`; the adapter code paths are real)
//! and asserts the honest `fail` at `"seam"`: 2 validation, 2
//! adversarial.
//!
//! The design's expected result here is the documented hole: "the gap
//! is detected by an explicit 'trace continuity' check — the task
//! fails until the propagation is fixed, documenting the hole."
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_66;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-66: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-66: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-66: HOME is not set"))
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
        "gauntlet-diver-rtp-66-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-66: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-66: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-66: cannot symlink ai: {e}"));
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
        "gauntlet-task-66-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-66: cannot build Ctx: {e}"));
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
            "task-66 passed: trace propagation was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the ACP round trip drives the
/// REAL acp adapter with a mock acp session peer and the peer receives
/// only (session_key, goal text) — no trace id. The honest verdict is
/// fail at "seam".
#[test]
fn acp_round_trip_drops_the_trace() {
    assert_eq!(task_66::ID, "task-66");
    assert_eq!(task_66::NAME, "trace propagation");
    assert_eq!(task_66::KIND, TaskKind::NvimLua);
    assert_eq!(task_66::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_at_seam(task_66::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-66 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("session.prompt received exactly 2 positional values"),
        "evidence must show the captured boundary-call shape:\n{joined}"
    );
    assert!(
        joined.contains("NO trace id"),
        "evidence must show the peer received no trace id:\n{joined}"
    );
    assert!(
        how.contains("session.prompt(session_key, goal_text)"),
        "the 'how' must name the real boundary-call shape: {how}"
    );
}

/// V: the A2A hop — the REAL a2a adapter's `tasks.submit` table carries
/// agent/message/timeout/on_done only. The design's A2A scenario (the
/// trace id survives message/send → tasks/get) cannot hold.
#[test]
fn a2a_hop_carries_no_trace_envelope() {
    let (where_, how, evidence) = fail_at_seam(task_66::run_scenario(&ctx_for(), "a2a-hop"));
    assert_eq!(where_, "seam", "task-66 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("submit keys:"),
        "evidence must show the captured submit table:\n{joined}"
    );
    assert!(
        joined.contains("no trace for tasks/get to echo"),
        "evidence must show the hop carries no trace:\n{joined}"
    );
    assert!(
        how.contains("no trace envelope"),
        "the 'how' must name the missing envelope: {how}"
    );
}

// --- adversarial ---

/// A: the explicit "trace continuity" check the design names — the
/// harness-issued trace id is compared against what each mock peer
/// received. Every comparison fails (2/2 hops drop it): the check
/// detects the gap, which IS the documented hole.
#[test]
fn continuity_check_detects_the_drop() {
    let (where_, _, evidence) = fail_at_seam(task_66::run_scenario(&ctx_for(), "trace-continuity"));
    assert_eq!(where_, "seam", "task-66 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("2/2 boundary hops dropped the trace id"),
        "evidence must show the continuity check detected the gap:\n{joined}"
    );
}

/// A: the contract check — every adapter's `probe()` contract is read
/// for a trace-propagation declaration or a declared inability. None
/// is found: the drops are silent, violating the design's "no silent
/// drops" criterion.
#[test]
fn adapters_declare_nothing_about_traces() {
    let (where_, _, evidence) = fail_at_seam(task_66::run_scenario(&ctx_for(), "contract-silence"));
    assert_eq!(where_, "seam", "task-66 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("probe() contract mentions trace: false"),
        "evidence must show the probe() contract reads:\n{joined}"
    );
    assert!(
        joined.contains("SILENT"),
        "evidence must name the silent drops:\n{joined}"
    );
}
