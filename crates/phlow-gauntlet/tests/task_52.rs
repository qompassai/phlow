//! Integration tests for task-52 (straggler mitigation).
//!
//! The seam is ABSENT: `ai.harness.supervisor` has no speculative
//! execution — no timeout launches a second attempt for a slow-but-
//! alive worker. Each test drives the `task_52.lua` behavioral probe
//! in headless Neovim against the REAL supervisor with a mock adapter
//! whose worker latency is driver-controlled: 2 validation, 2
//! adversarial. The task-level verdict is an honest `fail` at
//! `"seam"`; each scenario probe is expected to pass (the probe
//! characterizes the real no-speculation behavior correctly).
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_52;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-52: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-52: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-52: HOME is not set"))
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
        "gauntlet-diver-rtp-52-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-52: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-52: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-52: cannot symlink ai: {e}"));
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
        "gauntlet-task-52-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-52: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(180);
    ctx
}

/// Unwrap a passing probe verdict into its evidence, or panic with the
/// probe failure.
fn pass_evidence(outcome: TaskOutcome, scenario: &str) -> Vec<String> {
    match outcome {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            panic!("task-52 scenario '{scenario}' probe failed at '{where_}': {how}\n{evidence:?}")
        }
    }
}

/// Unwrap the expected task-level `fail` verdict, or panic.
fn fail_verdict(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-52 passed: speculation machinery was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the task-level verdict is an
/// honest `fail` at `"seam"` — the supervisor has no speculative
/// execution, and the aggregated probe evidence proves it behaviorally.
#[test]
fn task_reports_seam_absence() {
    assert_eq!(task_52::ID, "task-52");
    assert_eq!(task_52::NAME, "straggler mitigation");
    assert_eq!(task_52::KIND, TaskKind::NvimLua);
    assert_eq!(task_52::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_verdict(task_52::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-52 must fail at the absent seam");
    assert!(
        how.contains("no speculative execution"),
        "the 'how' must name the absent machinery: {how}"
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("scenario straggler: probe passed"),
        "evidence must aggregate all four probe scenarios:\n{joined}"
    );
}

/// V: the straggler probe — 2 fast workers complete, the slow worker is
/// left pending while the simulated clock is stepped 10x past the fast
/// path. The slow run keeps exactly 1 `run.started` (no speculative
/// duplicate) and completes at the straggler's pace.
#[test]
fn straggler_gets_no_speculative_duplicate() {
    let evidence = pass_evidence(task_52::run_scenario(&ctx_for(), "straggler"), "straggler");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no speculative duplicate launched"),
        "expected no duplicate after 10x the fast path:\n{joined}"
    );
    assert!(
        joined.contains("bounded by the slow worker, NOT by speculation-timeout"),
        "expected the p99 finding:\n{joined}"
    );
}

// --- adversarial ---

/// A: single-commit holds — after the straggler completes, exactly 1
/// `run.finished` exists and a second finish is refused as an invalid
/// transition. The design's "at most one result commits" is asserted
/// against real supervisor state (vacuously: one attempt was launched).
#[test]
fn single_commit_holds_and_double_finish_is_refused() {
    let evidence = pass_evidence(
        task_52::run_scenario(&ctx_for(), "single-commit"),
        "single-commit",
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("exactly 1 run.finished: one result committed"),
        "expected exactly one committed result:\n{joined}"
    );
    assert!(
        joined.contains("second finish refused"),
        "expected the duplicate finish to be refused:\n{joined}"
    );
}

/// A: the supervisor export table carries no speculation API — the
/// probe names the needles it scanned for and lists the real exports,
/// and the fail-closed arm would report `where = "recon"` if one ever
/// appeared. The probe must complete (not crash) for the absence claim
/// to mean anything.
#[test]
fn no_speculation_api_on_the_supervisor() {
    let evidence = pass_evidence(
        task_52::run_scenario(&ctx_for(), "no-speculation-api"),
        "no-speculation-api",
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("zero speculation-API hits"),
        "expected the zero-hit record:\n{joined}"
    );
    assert!(
        joined.contains("supervisor exports:"),
        "expected the real export list in evidence:\n{joined}"
    );
    assert!(
        joined.contains("retry_run"),
        "expected the real supervisor exports (create/start/finish/retry_run) listed:\n{joined}"
    );
}
