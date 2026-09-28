//! Integration tests for task-73 (subagent failure containment).
//!
//! The seam is ABSENT: diver's supervisor records failures but
//! quarantines nothing — `finish()` trusts terminal outcome strings
//! (never calling `verdict.evaluate`), no `subagent_unverified` /
//! `subagent_failed` distinction exists, no subtree cancellation API
//! exists, `cancel()` affects exactly one run, and the sink is global.
//! The failure IS recorded and siblings ARE unaffected — the recording
//! half works; the containment half does not. Each test drives the
//! `task_73.lua` probe in headless Neovim against the REAL supervisor
//! (holding/hostile fake adapters; runs are never started) and asserts
//! the honest `fail` at `"seam"`: 2 validation, 2 adversarial.
//!
//! The design's expected result here is the documented hole: failure
//! RECORDING exists; failure QUARANTINE does not. Diver-owned finding:
//! flagged, never fixed on gauntlet authority.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_73;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-73: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-73: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-73: HOME is not set"))
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
        "gauntlet-diver-rtp-73-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-73: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-73: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-73: cannot symlink ai: {e}"));
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
        "gauntlet-task-73-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-73: cannot build Ctx: {e}"));
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
        TaskOutcome::Pass { evidence } => {
            panic!("task-73 passed: containment was invented, not found\nevidence: {evidence:?}")
        }
    }
}

// --- validation ---

/// V: metadata contract pins the task; a failed child is recorded with
/// its cause chain while siblings are unaffected — the recording half
/// works. But the recorded outcome is a trusted string: `finish()`
/// never calls `verdict.evaluate` and no `subagent_unverified`
/// distinction exists. The honest verdict is fail at "seam".
#[test]
fn failure_is_recorded_but_never_verified() {
    assert_eq!(task_73::ID, "task-73");
    assert_eq!(task_73::NAME, "subagent failure containment");
    assert_eq!(task_73::KIND, TaskKind::NvimLua);
    assert_eq!(task_73::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_at_seam(task_73::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-73 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("finished failed with cause chain") && joined.contains("did not cascade"),
        "evidence must show recording works while the failure spreads nowhere:\n{joined}"
    );
    assert!(
        how.contains("verdict claims are never verified against evidence"),
        "the 'how' must name the trusted-string mechanism: {how}"
    );
    assert!(
        how.contains("finish() never calls verdict.evaluate"),
        "the 'how' must name the missing verification: {how}"
    );
}

/// V: a child hangs past its 50ms deadline with a live grandchild —
/// `tick()`'s `finish()` refuses with 'parent run owns live children';
/// no subtree-kill API exists (zero "subtree" mentions in
/// supervisor.lua); `M.cancel` on the child ORPHANS the grandchild.
/// The timed-out subtree is not reclaimed.
#[test]
fn hung_subtree_is_not_reclaimed() {
    let (where_, _, evidence) = fail_at_seam(task_73::run_scenario(&ctx_for(), "hang-no-reclaim"));
    assert_eq!(where_, "seam", "task-73 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("parent run owns live children"),
        "evidence must show finish() refusing the timed-out parent:\n{joined}"
    );
    assert!(
        joined.contains("subtree\" mentions = 0"),
        "evidence must show the source scan:\n{joined}"
    );
    assert!(
        joined.contains("orphaned"),
        "evidence must show the orphaned grandchild:\n{joined}"
    );
}

// --- adversarial ---

/// A: `finish(child, 'completed', 'subagent says so')` with ZERO
/// evidence artifacts lands in `completed`, indistinguishable from a
/// verified completion. The supervisor trusts the terminal outcome
/// string; the design's "verified against evidence" half does not
/// exist.
#[test]
fn false_success_is_indistinguishable_from_real() {
    let (where_, _, evidence) = fail_at_seam(task_73::run_scenario(&ctx_for(), "false-success"));
    assert_eq!(where_, "seam", "task-73 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("child.state = completed") && joined.contains("evidence-free finish"),
        "evidence must show the evidence-free completion:\n{joined}"
    );
    assert!(
        joined.contains("no subagent_unverified outcome")
            && joined.contains("trusts the outcome string"),
        "evidence must name the indistinguishability:\n{joined}"
    );
}

/// A: the hostile adapter's `model.completed` payload carrying an
/// injection string is merged VERBATIM into the run.finished record by
/// `drain_completions` — no quarantine layer, no sanitization, the
/// sink is global.
#[test]
fn hostile_output_is_not_quarantined() {
    let (where_, how, evidence) =
        fail_at_seam(task_73::run_scenario(&ctx_for(), "hostile-unfiltered"));
    assert_eq!(where_, "seam", "task-73 must fail at the absent seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("stored verbatim"),
        "evidence must show the verbatim merge:\n{joined}"
    );
    assert!(
        joined.contains("no quarantine, no sanitization"),
        "evidence must name the missing quarantine:\n{joined}"
    );
    assert!(
        how.contains("No quarantine or sanitization step exists"),
        "the 'how' must name the missing quarantine: {how}"
    );
}
