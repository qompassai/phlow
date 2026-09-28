//! Integration tests for task-75 (delegation cycle detection).
//!
//! The seam is ABSENT: diver's spawn path performs no ancestry-
//! membership check — no per-spawn O(depth) walk, no
//! `delegation_cycle` error, and the only "cycle" substring in the
//! loaded supervisor.lua sits inside the word "lifecycle". Strict graph cycles are structurally
//! unrepresentable (a parent_id must name an already-existing run; ids
//! are minted fresh at create), but that is construction, not a
//! check — and the design's checkable cases (escalation loops, upward
//! delegation, foreign parent_id) are all representable and ALL
//! accepted. The gauntlet itself runs the O(depth) ancestry walk the
//! design demands (iterative, capped, no recursion over
//! attacker-controlled depth) externally under live nvim and validates
//! it on linear and branching trees — but the walk lives only in the
//! gauntlet: the supervisor never runs it. The design's expected
//! result here is the documented hole. Diver-owned finding: flagged,
//! never fixed on gauntlet authority.
//!
//! Each test drives the `task_75.lua` probe in headless Neovim
//! against the REAL supervisor (mock sink + mock registry; runs are
//! never started) and asserts the honest `fail` at `"seam"`: 2
//! validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_75;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-75: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-75: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-75: HOME is not set"))
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
        "gauntlet-diver-rtp-75-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-75: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-75: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-75: cannot symlink ai: {e}"));
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
        "gauntlet-task-75-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-75: cannot build Ctx: {e}"));
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
            "task-75 passed: cycle detection was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; a linear chain of 5 works, and
/// the gauntlet's O(depth) ancestry walk (iterative, capped, no
/// recursion over attacker-controlled depth) yields 0..4 with no false
/// positives. But the walk is the GAUNTLET's, not the supervisor's: no
/// spawn ever checks membership in the requester's ancestry set. The
/// honest verdict is fail at "seam".
#[test]
fn linear_chain_walk_is_sound_but_unused_by_spawn() {
    assert_eq!(task_75::ID, "task-75");
    assert_eq!(task_75::NAME, "delegation cycle detection");
    assert_eq!(task_75::KIND, TaskKind::NvimLua);
    assert_eq!(task_75::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_at_seam(task_75::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-75 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("ancestry sets are 0,1,2,3,4"),
        "evidence must show the walked depths:\n{joined}"
    );
    assert!(
        how.contains("not the supervisor's"),
        "the 'how' must show the walk lives only in the gauntlet: {how}"
    );
    assert!(
        how.contains("no spawn ever checks membership"),
        "the 'how' must name the missing per-spawn check: {how}"
    );
}

/// V: the walk the design demands is validated on a branching tree —
/// 6 nodes, every ancestry set exact — under live nvim. The check is
/// O(depth), iterative, capped. But it exists only in the gauntlet:
/// `supervisor.create` / `spawn_child` never run it.
#[test]
fn ancestry_walk_validated_on_branching_tree() {
    let (where_, _, evidence) = fail_at_seam(task_75::run_scenario(&ctx_for(), "walk-sound"));
    assert_eq!(where_, "seam", "task-75 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("every ancestry set exact"),
        "evidence must show the walk was validated:\n{joined}"
    );
    assert!(
        joined.contains("the supervisor never runs it"),
        "evidence must show the walk lives only in the gauntlet:\n{joined}"
    );
}

// --- adversarial ---

/// A: the design's "escalation loop" — B (child of A) delegates back
/// UP to its ancestor A under a different workflow name — is ACCEPTED
/// silently. The design wants this rejected BY IDENTITY (not by name)
/// with `delegation_cycle` naming the cycle. No such rejection exists.
#[test]
fn escalation_loop_is_accepted_silently() {
    let (where_, how, evidence) =
        fail_at_seam(task_75::run_scenario(&ctx_for(), "escalation-accepted"));
    assert_eq!(where_, "seam", "task-75 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("the ANCESTOR") && joined.contains("totally-different-workflow"),
        "evidence must show the upward delegation under a different name:\n{joined}"
    );
    assert!(
        joined.contains("accepted silently"),
        "evidence must show the silent acceptance:\n{joined}"
    );
    assert!(
        how.contains("identity-based delegation_cycle detection has no implementation"),
        "the 'how' must name the missing identity-based check: {how}"
    );
}

/// A: the spawn path contains no cycle machinery at all — the only
/// "cycle" substring in the loaded supervisor.lua sits inside the word
/// "lifecycle", no `delegation_cycle` string — and the ancestry set is
/// never consulted: a spawn naming an unrelated live run as parent_id
/// is accepted. Strict cycles are structurally unrepresentable (fresh
/// ids), but that is construction, not a check.
#[test]
fn spawn_path_has_no_cycle_machinery() {
    let (where_, _, evidence) =
        fail_at_seam(task_75::run_scenario(&ctx_for(), "no-cycle-machinery"));
    assert_eq!(where_, "seam", "task-75 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("\"lifecycle\" occurrences = 1"),
        "evidence must show the source scan:\n{joined}"
    );
    assert!(
        joined.contains("no cycle DETECTION machinery"),
        "evidence must name the missing detection machinery:\n{joined}"
    );
    assert!(
        joined.contains("caller-asserted, never checked"),
        "evidence must show the ancestry set is never consulted:\n{joined}"
    );
}
