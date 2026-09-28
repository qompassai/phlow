//! Integration tests for task-92 (proposal scope binding and drift detection).
//!
//! The lua seam is ABSENT as designed: diver's `ai.harness.approval` is
//! a data-only queue with no content-hash binding API; `ai.harness.policy`
//! binds approvals to action scope (rule decisions — task-59's
//! mechanism), never to reviewed bytes; `ai.harness.store`'s
//! `content_hash` hashes artifact bytes for dedup and is never
//! referenced by the approval queue. No drift-detection module exists,
//! no `proposal_drift` typed error exists, no re-review/re-approve
//! path exists.
//!
//! Each test drives the `task_92.lua` probe in headless Neovim against
//! the REAL `ai.harness` modules and asserts the honest `fail` at
//! `"seam"`: 2 validation, 2 adversarial.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//! (The rust half of this design point is task-91's A1:
//! phlow-experiment's gate never compares the approval's candidate
//! digest against the proposal.)
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`,
//! with fallbacks to Matt's known tool paths. Missing binaries or
//! directories panic with a clear message: the gauntlet fails closed,
//! never skips.

use phlow_gauntlet::tasks::task_92;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-92: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-92: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-92: HOME is not set"))
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
        "gauntlet-diver-rtp-92-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-92: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-92: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-92: cannot symlink ai: {e}"));
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
        "gauntlet-task-92-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-92: cannot build Ctx: {e}"));
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
            "task-92 passed: an approval→content-hash binding was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: the approval queue exposes no content-hash binding API, and the
/// approval record carries no hash field. `ai.harness.store`'s
/// `content_hash` is artifact dedup, unreachable from the approval
/// queue.
#[test]
fn no_content_hash_binding() {
    assert_eq!(task_92::ID, "task-92");
    assert_eq!(task_92::NAME, "proposal scope binding and drift detection");
    assert_eq!(task_92::KIND, TaskKind::NvimLua);
    assert_eq!(task_92::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_92::run_case(&ctx_for(), "no_content_hash_binding")
        .unwrap_or_else(|e| panic!("task-92 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-binding case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["binding_api"], false);
    assert_eq!(report.metrics["record_has_hash"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("binding_api=Some(false)"),
        "evidence must show the absent binding API:\n{joined}"
    );
}

/// V2: the policy binds approvals to action scope (rule decisions —
/// the task-59 tool-use mechanism), not to content hashes. The
/// task-level driver then fails at the seam: the approval→content-hash
/// binding is absent as designed.
#[test]
fn scope_not_hash() {
    let report = task_92::run_case(&ctx_for(), "scope_not_hash")
        .unwrap_or_else(|e| panic!("task-92 case failed to run: {e}"));
    assert!(
        report.passed,
        "scope case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["scope_binding"], true);
    assert_eq!(report.metrics["hash_binding"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("hash_binding=Some(false)"),
        "evidence must show the absent hash binding:\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass).
    let (where_, how, _) = fail_at_seam(task_92::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-92 must fail at the seam");
    assert!(
        how.contains("seam absent"),
        "the 'how' must name the absent seam: {how}"
    );
    assert!(
        how.contains("Diver-owned"),
        "the 'how' must flag diver ownership: {how}"
    );
}

// --- adversarial ---

/// A1 (harness probe): no drift-detection module or API exists in
/// lua/ai — the candidate modules are all absent and the token scan
/// for `drift` finds zero hits. A rebase or concurrent edit between
/// approval and apply would not be detected lua-side.
#[test]
fn no_drift_detection() {
    let report = task_92::run_case(&ctx_for(), "no_drift_detection")
        .unwrap_or_else(|e| panic!("task-92 case failed to run: {e}"));
    assert!(
        report.passed,
        "drift case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["drift_hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("drift_hits=Some(0)"),
        "evidence must show the zero-hit scan:\n{joined}"
    );
}

/// A2 (harness probe): no re-review/re-approve recovery path exists —
/// the token scan for `re_approve`/`reapprove`/`proposal_drift` finds
/// zero hits. Fail-closed would be fail-stuck: a drifted proposal has
/// no lua-side path back to approval.
#[test]
fn no_recovery_path() {
    let report = task_92::run_case(&ctx_for(), "no_recovery_path")
        .unwrap_or_else(|e| panic!("task-92 case failed to run: {e}"));
    assert!(
        report.passed,
        "recovery case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["recovery_hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("recovery_hits=Some(0)"),
        "evidence must show the zero-hit scan:\n{joined}"
    );
}
