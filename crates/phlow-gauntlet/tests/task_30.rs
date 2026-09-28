//! Integration tests for task-30 (leader election under partition).
//!
//! The seam is ABSENT: diver's coordination modules
//! (`ai.harness.supervisor`, `ai.harness.adapters.herd`, `ai.herd`,
//! `ai.herd.api`) implement task coordination, not leader election —
//! no module elects a leader, detects leader death, or handles
//! partitions. Each test drives the `task_30.lua` recon probe in
//! headless Neovim against the REAL diver Lua tree and asserts aspects
//! of the honest `fail` verdict (`where = "seam"`): 2 validation, 2
//! adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_30;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-30: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-30: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-30: HOME is not set"))
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
        "gauntlet-task-30-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-30: cannot build Ctx: {e}"));
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
            "task-30 passed: leader election machinery was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the recon probe completes and
/// reports the seam absence — `where = "seam"`, naming the missing
/// leader-election machinery.
#[test]
fn probe_reports_seam_absence() {
    assert_eq!(task_30::ID, "task-30");
    assert_eq!(task_30::NAME, "leader election under partition");
    assert_eq!(task_30::KIND, TaskKind::NvimLua);
    let (where_, how, _evidence) = fail_verdict(task_30::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-30 must fail at the absent seam");
    assert!(
        how.contains("no leader election machinery"),
        "the 'how' must name the absent machinery: {how}"
    );
}

/// V: the probe covered every coordination module the design names —
/// supervisor, herd adapter, and both herd modules — before concluding.
#[test]
fn probe_covers_all_design_named_modules() {
    let (_where_, _how, evidence) = fail_verdict(task_30::run(&ctx_for()));
    let joined = evidence.join("\n");
    for module in [
        "ai.harness.supervisor",
        "ai.harness.adapters.herd",
        "ai.herd",
        "ai.herd.api",
    ] {
        assert!(
            joined.contains(module),
            "evidence must show {module} was probed:\n{joined}"
        );
    }
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_30::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: the probe explicitly records zero election-API hits across all
/// probed modules, and states the open design gap — the "at most one
/// leader" invariant has no seam. The evidence must say what was looked
/// for, not just what was found.
#[test]
fn zero_election_hits_recorded_explicitly() {
    let (_where_, how, evidence) = fail_verdict(task_30::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no election APIs"),
        "evidence must record the zero-hit result explicitly:\n{joined}"
    );
    assert!(
        joined.contains("elect"),
        "evidence must show which API names were scanned for:\n{joined}"
    );
    assert!(
        how.contains("open design gap"),
        "the 'how' must mark this as an open gap, not a driver error: {how}"
    );
}
