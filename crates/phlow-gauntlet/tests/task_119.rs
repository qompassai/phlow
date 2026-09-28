//! Integration tests for task-119 (resume mutation ordering, diver Fix 5).
//!
//! The defect: `supervisor.resume` (supervisor.lua line 313) mutates the
//! run (attempt, generation, _terminal_emitted, handle) BEFORE validating
//! the queued transition. Resuming a completed run corrupts the run table,
//! then reports the invalid transition — the corruption is never repaired.
//!
//! The tests assert the honest defect evidence: the completed-resume
//! scenario reports `fail` with `where = "fix-5-absent"` (the completed run
//! is corrupted before the error); the default and generation scenarios
//! pass weakly today (re-queue works, but ordering is unverifiable from
//! outside); running-rejection passes for the rejection half. 50/50 split:
//! 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_119;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-119: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-119: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-119: HOME is not set"))
}

/// Process-local sequence so concurrent `ctx_for` calls never collide.
static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build a `Ctx` for one scenario with its own scratch directory.
///
/// The workdir is unique per call (pid + a process-local counter): two
/// tests running the *same* scenario in parallel get disjoint directories,
/// while the driver still sees the unchanged scenario name.
fn ctx_for(scenario: &str) -> Ctx {
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
        "gauntlet-task-119-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-119: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Driver errors are harness failures, not findings: anything that is not
/// an honest acceptance verdict fails the test loudly.
enum Probe {
    Fail {
        where_: String,
        how: String,
        evidence: Vec<String>,
    },
    Pass {
        evidence: Vec<String>,
    },
}

fn probe(scenario: &str) -> Probe {
    let ctx = ctx_for(scenario);
    match task_119::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => Probe::Fail {
            where_,
            how,
            evidence,
        },
        TaskOutcome::Pass { evidence } => Probe::Pass { evidence },
    }
}

// --- validation ---

/// V: metadata contract pins the task, and resuming a failed run
/// re-queues it with attempt+1 and generation+1. The pass is weak: the
/// ordering between validation and mutation is unverifiable from outside.
#[test]
fn failed_run_resumes_with_clean_counters() {
    assert_eq!(task_119::ID, "task-119");
    assert_eq!(task_119::NAME, "resume-mutation-ordering");
    assert_eq!(task_119::KIND, TaskKind::NvimLua);
    let evidence = match probe("default") {
        Probe::Pass { evidence } => evidence,
        Probe::Fail { how, .. } => {
            panic!("task-119 'default' failed: even the basic failed-run resume broke: {how}")
        }
    };
    let joined = evidence.join("\n");
    assert!(
        joined.contains("attempt+1") || joined.contains("attempt"),
        "evidence must show attempt advanced:\\n{joined}"
    );
    assert!(
        joined.contains("unverifiable from outside") || joined.contains("weakly"),
        "evidence must record that ordering is unverifiable from outside:\\n{joined}"
    );
}

/// V: cancelling a resumed run bumps the generation so stale adapter
/// callbacks are dropped — the generation mechanism itself works.
#[test]
fn cancel_after_resume_invalidates_old_generation() {
    let evidence = match probe("generation-stale") {
        Probe::Pass { evidence } => evidence,
        Probe::Fail { how, .. } => {
            panic!("task-119 'generation-stale' failed: generation invalidation broke: {how}")
        }
    };
    let joined = evidence.join("\n");
    assert!(
        joined.contains("generation"),
        "evidence must cover the generation bump:\\n{joined}"
    );
}

// --- adversarial ---

/// A: resuming a completed run reports invalid_transition but the run
/// table was already mutated — the corruption the fix must prevent.
#[test]
fn completed_resume_corrupts_before_rejecting() {
    let (how, evidence) = match probe("completed-resume") {
        Probe::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "fix-5-absent",
                "task-119 'completed-resume' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        Probe::Pass { evidence } => panic!(
            "task-119 'completed-resume' passed: completed runs are rejected without corruption, contradicting the probed defect\\nevidence: {evidence:?}"
        ),
    };
    let joined = evidence.join("\n");
    assert!(
        joined.contains("MUTATED") || joined.contains("mutat"),
        "evidence must show the run table was mutated before the error:\\n{joined}"
    );
    assert!(
        how.contains("validate first") || how.contains("validate"),
        "the 'how' must demand validate-before-mutate: {how}"
    );
}

/// A: resuming a running run is rejected (docs: only terminal runs
/// resume), and double resume on a failed run advances attempt
/// monotonically (+1 per resume) without double-counting generation.
#[test]
fn running_resume_rejected_and_double_resume_monotonic() {
    let evidence = match probe("running-rejection") {
        Probe::Pass { evidence } => evidence,
        Probe::Fail { how, .. } => {
            panic!("task-119 'running-rejection' failed: the rejection/monotonicity broke: {how}")
        }
    };
    let joined = evidence.join("\n");
    assert!(
        joined.contains("rejected"),
        "evidence must show the running resume was rejected:\\n{joined}"
    );
    assert!(
        joined.contains("monotonic") || joined.contains("attempt"),
        "evidence must cover double-resume monotonicity:\\n{joined}"
    );
}
