//! Integration tests for task-62 (replanning on partial failure).
//!
//! The seam is ABSENT: diver has no replan entry point. The
//! supervisor's recovery vocabulary is `resume` (re-launch the SAME
//! run_id — same spec, same adapter) and `retry_run` (bounded same-run
//! retry with backoff and an attempt ceiling, re-queueing the same run
//! to `retry_wait`); neither emits a new plan for the remainder and
//! neither is semantic about which completed steps stay valid. Each test
//! drives the `task_62.lua` probe in headless Neovim against the REAL
//! diver Lua tree and asserts aspects of the honest `fail` verdict
//! (`where = "seam"`): 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_62;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-62: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-62: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-62: HOME is not set"))
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
        "gauntlet-diver-rtp-62-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-62: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-62: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-62: cannot symlink ai: {e}"));
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
        "gauntlet-task-62-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-62: cannot build Ctx: {e}"));
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
        TaskOutcome::Pass { evidence } => {
            panic!(
                "task-62 passed: replan machinery was invented, not found\nevidence: {evidence:?}"
            )
        }
    }
}

// --- validation ---

/// V: metadata contract pins the task; the replan entry-point scan
/// completes and reports the absent seam — `where = "seam"` — with the
/// `how` documenting that the harness only supports resume.
#[test]
fn probe_reports_absent_seam() {
    assert_eq!(task_62::ID, "task-62");
    assert_eq!(task_62::NAME, "replanning on partial failure");
    assert_eq!(task_62::KIND, TaskKind::NvimLua);
    assert_eq!(task_62::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_verdict(task_62::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-62 must fail at the absent seam");
    assert!(
        how.contains("harness only supports resume"),
        "the 'how' must document the resume-only gap: {how}"
    );
    let joined_ev = evidence.join("\n");
    assert!(
        joined_ev.contains("zero replan hits"),
        "evidence must state the scan found nothing:\n{joined_ev}"
    );
}

/// V: the resume facet — `M.resume`'s body (read from the real
/// supervisor.lua) re-launches the SAME run_id with the SAME adapter and
/// contains no plan token: resume is not replan.
#[test]
fn resume_relaunches_same_run() {
    let (where_, _how, evidence) = fail_verdict(task_62::run_scenario(
        &ctx_for(),
        "resume-relaunches-same-run",
    ));
    assert_eq!(
        where_, "seam",
        "the resume body must not flip the verdict to 'recon'"
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("launch(sup, run, run.adapter)"),
        "evidence must show resume re-launches the same run and adapter:\n{joined}"
    );
    assert!(
        joined.contains("no plan token"),
        "evidence must show the resume body constructs no plan:\n{joined}"
    );
}

// --- adversarial ---

/// A: the retry facet — `M.retry_run`'s body re-queues the SAME run to
/// `retry_wait` with an attempt ceiling and contains no plan token: a
/// step-2 failure can never produce "plan v2 covering steps 3-5 with a
/// workaround", and no semantic re-evaluation of completed steps exists.
#[test]
fn retry_keeps_same_run() {
    let (where_, _how, evidence) =
        fail_verdict(task_62::run_scenario(&ctx_for(), "retry-keeps-same-run"));
    assert_eq!(
        where_, "seam",
        "bounded retry must not masquerade as replan"
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("retry attempt ceiling exceeded"),
        "evidence must show the bounded attempt ceiling:\n{joined}"
    );
    assert!(
        joined.contains("no semantic re-evaluation"),
        "evidence must state retry is not semantic:\n{joined}"
    );
}

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_62::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}
