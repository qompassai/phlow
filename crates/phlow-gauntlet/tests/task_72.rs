//! Integration tests for task-72 (context handoff fidelity).
//!
//! The seam is ABSENT: diver's spawn path has NO handoff envelope —
//! the child run record is the spawn spec minus the goal (validated
//! then dropped), plus a FRESH budget (never the parent's remainder);
//! there is no per-run tool allowlist (policy allowlists are
//! supervisor-global); and the spec has no size bound. Each test
//! drives the `task_72.lua` probe in headless Neovim against the REAL
//! supervisor (mock sink + mock registry; runs are never started) and
//! asserts the honest `fail` at `"seam"`: 2 validation, 2 adversarial.
//!
//! The design's expected result here is the documented hole: there is
//! no envelope to be faithful to, so every fidelity property fails at
//! the seam. Diver-owned finding: flagged, never fixed on gauntlet
//! authority.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_72;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-72: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-72: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-72: HOME is not set"))
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
        "gauntlet-diver-rtp-72-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-72: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-72: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-72: cannot symlink ai: {e}"));
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
        "gauntlet-task-72-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-72: cannot build Ctx: {e}"));
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
            "task-72 passed: a handoff envelope was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the child run record's fields
/// are enumerated — and `spec.goal`, though REQUIRED non-empty by
/// `validate_run_spec`, is dropped from the run table. The delegation
/// boundary does not even carry the goal. The honest verdict is fail
/// at "seam".
#[test]
fn handoff_carries_no_envelope_and_drops_the_goal() {
    assert_eq!(task_72::ID, "task-72");
    assert_eq!(task_72::NAME, "context handoff fidelity");
    assert_eq!(task_72::KIND, TaskKind::NvimLua);
    assert_eq!(task_72::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_at_seam(task_72::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-72 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("child run record fields:"),
        "evidence must enumerate the run record fields:\n{joined}"
    );
    assert!(
        joined.contains("run.goal is ABSENT"),
        "evidence must show the goal was validated then dropped:\n{joined}"
    );
    assert!(
        how.contains("no handoff envelope"),
        "the 'how' must name the missing envelope: {how}"
    );
}

/// V: the parent spends token budget; children spawned without an
/// explicit budget get FULL defaults with zero used — never the
/// parent's remainder. Two siblings each get full defaults: the
/// shared remainder the design demands does not exist.
#[test]
fn budget_remainder_is_not_handed_off() {
    let (where_, _, evidence) = fail_at_seam(task_72::run_scenario(&ctx_for(), "budget-not-split"));
    assert_eq!(where_, "seam", "task-72 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("sibling A budget: used=0") && joined.contains("sibling B budget: used=0"),
        "evidence must show both siblings got fresh full budgets:\n{joined}"
    );
    assert!(
        joined.contains("budget.new(spec.budget or DEFAULT_BUDGET_LIMITS)"),
        "evidence must name the fresh-budget mechanism:\n{joined}"
    );
}

// --- adversarial ---

/// A: a 10MB `extensions.blob` is stored on the child run silently —
/// no bound, no explicit error, no truncation. The design demands
/// oversized handoffs "fail explicitly, not silent truncation".
#[test]
fn oversized_handoff_is_silently_accepted() {
    let (where_, _, evidence) = fail_at_seam(task_72::run_scenario(&ctx_for(), "oversized-blob"));
    assert_eq!(where_, "seam", "task-72 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("10485760 bytes round-tripped"),
        "evidence must show the full 10MB was stored:\n{joined}"
    );
    assert!(
        joined.contains("silent unbounded acceptance"),
        "evidence must name the silent acceptance:\n{joined}"
    );
}

/// A: allowlist narrowing is unrepresentable at spawn — policy
/// allowlists are supervisor-global rules, the spawn spec has no
/// allowlist field, and `spawn_child` performs no narrowing step. A
/// widened child cannot be rejected at spawn.
#[test]
fn allowlist_narrowing_is_unrepresentable() {
    let (where_, how, evidence) = fail_at_seam(task_72::run_scenario(&ctx_for(), "no-allowlist"));
    assert_eq!(where_, "seam", "task-72 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no such field exists"),
        "evidence must show the child carries no allowlist:\n{joined}"
    );
    assert!(
        joined.contains("nothing was narrowed for the child"),
        "evidence must show the global policy was untouched by spawn:\n{joined}"
    );
    assert!(
        how.contains("unrepresentable at spawn"),
        "the 'how' must name the unrepresentable narrowing: {how}"
    );
}
