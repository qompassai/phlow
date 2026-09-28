//! Integration tests for task-71 (delegation depth attribution).
//!
//! The seam is ABSENT: diver's supervisor keeps a run TREE
//! (`parent_id`/`root_id`) but `M.create` / `M.spawn_child` never walk
//! it, compute no depth, enforce no depth bound, and verify nothing
//! about the asserted parent_id beyond "exists and non-terminal". Each
//! test drives the `task_71.lua` probe in headless Neovim against the
//! REAL supervisor (mock sink + mock registry; runs are never started)
//! and asserts the honest `fail` at `"seam"`: 2 validation, 2
//! adversarial.
//!
//! The design's expected result here is the documented hole: depth is
//! computable from the supervisor's own parent_id tree, but the
//! supervisor never computes it — the attribution-integrity attack (a
//! child lying about where it sits in the tree) meets no check.
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_71;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-71: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-71: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-71: HOME is not set"))
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
        "gauntlet-diver-rtp-71-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-71: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-71: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-71: cannot symlink ai: {e}"));
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
        "gauntlet-task-71-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-71: cannot build Ctx: {e}"));
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
            "task-71 passed: depth attribution was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the parent→child→grandchild
/// chain walks to depths 0/1/2 from the supervisor's own parent_id
/// tree — the DATA for supervisor-side attribution exists — but no
/// run carries a supervisor-computed depth field. The honest verdict
/// is fail at "seam".
#[test]
fn chain_depth_is_supervisor_computable_but_uncomputed() {
    assert_eq!(task_71::ID, "task-71");
    assert_eq!(task_71::NAME, "delegation depth attribution");
    assert_eq!(task_71::KIND, TaskKind::NvimLua);
    assert_eq!(task_71::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_at_seam(task_71::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-71 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("depths 0/1/2"),
        "evidence must show the walked depths:\n{joined}"
    );
    assert!(
        joined.contains("not by supervisor.create/spawn_child"),
        "evidence must show the supervisor never computes depth:\n{joined}"
    );
    assert!(
        how.contains("never compute it"),
        "the 'how' must name the missing supervisor computation: {how}"
    );
}

/// V: a 30-deep linear chain spawns with zero resistance — no depth
/// bound is enforced at spawn time; the only bound is the total-run
/// cap. The design's "bound enforced at spawn time" is absent.
#[test]
fn deep_chain_meets_no_depth_bound() {
    let (where_, _, evidence) = fail_at_seam(task_71::run_scenario(&ctx_for(), "depth-unbounded"));
    assert_eq!(where_, "seam", "task-71 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("tip depth = 30"),
        "evidence must show the 30-deep chain succeeded:\n{joined}"
    );
    assert!(
        joined.contains("no delegation_depth_exceeded exists"),
        "evidence must show the missing typed error:\n{joined}"
    );
}

// --- adversarial ---

/// A: the attribution-integrity attack — the child forges
/// `extensions.claimed_depth = 0` (the lie passes through unread; the
/// true ancestry depth is 2) and a spawn names a FOREIGN parent_id (an
/// unrelated live run — accepted silently). Attribution is
/// caller-asserted, never supervisor-verified.
#[test]
fn forged_depth_and_foreign_parent_are_accepted() {
    let (where_, how, evidence) = fail_at_seam(task_71::run_scenario(&ctx_for(), "forged-depth"));
    assert_eq!(where_, "seam", "task-71 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("claimed_depth = 0") && joined.contains("ancestry depth = 2"),
        "evidence must contrast the forged claim with the true depth:\n{joined}"
    );
    assert!(
        joined.contains("SUCCEEDED"),
        "evidence must show the foreign parent_id was accepted:\n{joined}"
    );
    assert!(
        how.contains("caller-asserted"),
        "the 'how' must name the caller-asserted attribution: {how}"
    );
}

/// A: the spawn path contains no depth machinery at all — zero
/// "depth" mentions in the loaded supervisor.lua — and a chain to the
/// cap is refused only with 'supervisor run bound exceeded'. No
/// rejection ever names an ancestry chain.
#[test]
fn spawn_path_has_no_depth_machinery() {
    let (where_, _, evidence) = fail_at_seam(task_71::run_scenario(&ctx_for(), "no-depth-error"));
    assert_eq!(where_, "seam", "task-71 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("\"depth\" mentions = 0"),
        "evidence must show the source scan:\n{joined}"
    );
    assert!(
        joined.contains("supervisor run bound exceeded"),
        "evidence must show the only refusal is the total-run cap:\n{joined}"
    );
}
