//! Integration tests for task-40 (approval TOCTOU).
//!
//! The seam is ABSENT as designed: diver's `ai.harness.approval` records
//! bind no approved-against state (no hash/digest/fingerprint), there is
//! no revocation API (an approved record can never move back), and no
//! execution-time re-validation exists — nothing in the harness consumes
//! an approval at execution; the supervisor only sweeps expiries. The
//! design's "executor re-validates the target" and "execution checks
//! liveness" have no seam to attach to. Each test drives the
//! `task_40.lua` probe in headless Neovim against the REAL diver Lua tree
//! and asserts aspects of the honest `fail` verdict (`where = "seam"`):
//! 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.
//!
//! Diver-owned finding: flagged in the probe evidence, never fixed on
//! gauntlet authority.

use phlow_gauntlet::tasks::task_40;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-40: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-40: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-40: HOME is not set"))
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
        "gauntlet-task-40-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-40: cannot build Ctx: {e}"));
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
            "task-40 passed: approval re-validation was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the probe completes and reports
/// the seam absence — `where = "seam"`, naming the missing
/// execution-time re-validation of approvals.
#[test]
fn probe_reports_seam_absence() {
    assert_eq!(task_40::ID, "task-40");
    assert_eq!(task_40::NAME, "approval TOCTOU");
    assert_eq!(task_40::KIND, TaskKind::NvimLua);
    let (where_, how, _evidence) = fail_verdict(task_40::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-40 must fail at the absent seam");
    assert!(
        how.contains("re-validation"),
        "the 'how' must name the missing re-validation: {how}"
    );
}

/// V: the probe exercised the real approval API — request, decide,
/// get round-trip — before concluding.
#[test]
fn probe_exercises_the_real_approval_api() {
    let (_where_, _how, evidence) = fail_verdict(task_40::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("decided approved, reads back approved"),
        "evidence must show the default grant scenario ran:\n{joined}"
    );
    assert!(
        joined.contains("approval record keys:"),
        "evidence must show the record shape was inspected:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_40::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: revocation is impossible and the record binds no state — the
/// revoke attempt fails with "approval is already approved", the record
/// keys show no state-hash field, and no execution-time liveness check
/// exists, so a grant-then-revoke race is invisible to the record.
#[test]
fn revocation_is_impossible_and_state_is_unbound() {
    let (_where_, how, evidence) = fail_verdict(task_40::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("approval is already approved"),
        "evidence must show the revoke attempt was rejected:\n{joined}"
    );
    assert!(
        joined.contains("binds NO state"),
        "evidence must show the record binds no approved-against state:\n{joined}"
    );
    assert!(
        joined.contains("no liveness check exists"),
        "evidence must show execution-time liveness has no seam:\n{joined}"
    );
    assert!(
        how.contains("no revocation"),
        "the 'how' must name the missing revocation path: {how}"
    );
}
