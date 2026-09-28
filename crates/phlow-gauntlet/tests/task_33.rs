//! Integration tests for task-33 (checkpoint durability).
//!
//! The seam is ABSENT as designed: diver's `ai.harness.store` checkpoint
//! API works, but persistence is IN-MEMORY ONLY ("Phase 1 store is
//! in-memory ... SQLite backing is Phase 5 work"). A SIGKILLed
//! supervisor takes its store with it — a new supervisor process builds
//! a fresh `M.new()` with empty tables, so `get_checkpoint` returns nil
//! and restore is impossible. Kill-during-write atomicity is vacuous (no
//! disk writes: the scratch dir stays empty) and checkpoint records
//! (`{ label, at_ns, state }`) carry no schema version, so no typed
//! rejection is possible. Each test drives the `task_33.lua` probe in
//! headless Neovim against the REAL diver Lua tree and asserts aspects
//! of the honest `fail` verdict (`where = "seam"`): 2 validation, 2
//! adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.
//!
//! Diver-owned finding: flagged in the probe evidence, never fixed on
//! gauntlet authority.

use phlow_gauntlet::tasks::task_33;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-33: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-33: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-33: HOME is not set"))
}

/// Process-local sequence so concurrent `ctx_for` calls never collide.
static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build a `Ctx` with its own scratch directory. The workdir is unique
/// per call (pid + a process-local counter): tests running in parallel
/// get disjoint directories.
fn ctx_for() -> Ctx {
    let nvim_bin = required_dir(
        "GAUNTLET_NVIM_BIN",
        &format!("{}/workspace/tools/neovim-nightly/bin/nvim", home_dir()),
    );
    let diver_lua = required_dir(
        "GAUNTLET_DIVER_LUA",
        &format!("{}/workspace/repos/diver/lua", home_dir()),
    );
    let seq = WORKDIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let work_dir = std::env::temp_dir().join(format!(
        "gauntlet-task-33-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-33: cannot build Ctx: {e}"));
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
            "task-33 passed: checkpoint durability was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the probe completes and reports
/// the seam absence — `where = "seam"`, naming the in-memory-only
/// persistence.
#[test]
fn probe_reports_seam_absence() {
    assert_eq!(task_33::ID, "task-33");
    assert_eq!(task_33::NAME, "checkpoint durability");
    assert_eq!(task_33::KIND, TaskKind::NvimLua);
    let (where_, how, _evidence) = fail_verdict(task_33::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-33 must fail at the absent seam");
    assert!(
        how.contains("in-memory"),
        "the 'how' must name the in-memory-only persistence: {how}"
    );
}

/// V: the probe exercised the real checkpoint API — save, checkpoint at
/// two step boundaries, get_checkpoint round-trip — before concluding.
/// The checkpoint API itself works; durability is what is missing.
#[test]
fn probe_exercises_the_real_checkpoint_api() {
    let (_where_, _how, evidence) = fail_verdict(task_33::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("step-1") && joined.contains("step-2"),
        "evidence must show checkpoints at two step boundaries:\n{joined}"
    );
    assert!(
        joined.contains("round-trip") || joined.contains("round-trips"),
        "evidence must show the in-memory round-trip worked:\n{joined}"
    );
    assert!(
        joined.contains("post-mortem"),
        "evidence must show the post-mortem (fresh store) check ran:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_33::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: kill-during-write atomicity is vacuous because checkpointing
/// writes nothing to disk — the probe asserts the scratch dir holds 0
/// files after checkpointing. Durability is zero, and the evidence says
/// so instead of claiming an atomic-write property that was never
/// tested against a real write.
#[test]
fn checkpoint_writes_no_disk_artifact() {
    let (_where_, how, evidence) = fail_verdict(task_33::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("0 files"),
        "evidence must record the empty scratch dir explicitly:\n{joined}"
    );
    assert!(
        joined.contains("no schema version"),
        "evidence must record the missing schema version:\n{joined}"
    );
    assert!(
        how.contains("no schema version"),
        "the 'how' must name the missing typed rejection: {how}"
    );
}
