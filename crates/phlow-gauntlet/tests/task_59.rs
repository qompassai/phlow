//! Integration tests for task-59 (approval scope binding).
//!
//! The seam is HALF-absent: the approval RECORD binds the exact action
//! and arguments (tool/argv/paths/endpoints stored verbatim — the
//! structure half is real), but no EXECUTOR exists in the harness to
//! verify the binding — the only approval consumer is `supervisor.tick`
//! → `sweep_expired` (expiry). The design's replay rejections cannot be
//! demonstrated against an executor that does not exist. Each test
//! drives the `task_59.lua` probe in headless Neovim against the REAL
//! diver Lua tree and asserts aspects of the honest `fail` verdict
//! (`where = "seam"`): 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_59;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-59: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-59: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-59: HOME is not set"))
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
        "gauntlet-diver-rtp-59-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-59: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-59: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-59: cannot symlink ai: {e}"));
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
        "gauntlet-task-59-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-59: cannot build Ctx: {e}"));
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
            "task-59 passed: executor verification was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the record-binds-action probe
/// completes and reports the seam absence — `where = "seam"` — while
/// its evidence shows the structure half is real: the record carries
/// the exact tool/argv/paths/endpoints.
#[test]
fn probe_reports_half_absent_seam() {
    assert_eq!(task_59::ID, "task-59");
    assert_eq!(task_59::NAME, "approval scope binding");
    assert_eq!(task_59::KIND, TaskKind::NvimLua);
    assert_eq!(task_59::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_verdict(task_59::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-59 must fail at the half-absent seam");
    let joined_ev = evidence.join("\n");
    assert!(
        joined_ev.contains("record binds the action"),
        "evidence must show the present structure half:\n{joined_ev}"
    );
    assert!(
        joined_ev.contains("tool=fs.write"),
        "evidence must show the bound action fields:\n{joined_ev}"
    );
    assert!(
        how.contains("no executor exists"),
        "the 'how' must name the absent executor: {how}"
    );
}

/// V: the binding-fields facet — argv/paths are stored verbatim with
/// element counts preserved, so a replay check would have the full
/// action shape to compare against, if an executor existed.
#[test]
fn binding_fields_stored_verbatim() {
    let (where_, _how, evidence) =
        fail_verdict(task_59::run_scenario(&ctx_for(), "binding-fields-verbatim"));
    assert_eq!(
        where_, "seam",
        "verbatim binding fields must not flip the verdict to 'recon'"
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("stored verbatim"),
        "evidence must show verbatim storage:\n{joined}"
    );
    assert!(
        joined.contains("if an executor existed"),
        "evidence must state the conditional honestly:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_59::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: the executor hunt — the only approval consumer in the harness is
/// `supervisor.tick` → `sweep_expired` (expiry, not execution), and the
/// replay facet documents both replay scenarios as uncheckable: with no
/// executor, nothing can reject approval-for-X presented for action Y.
#[test]
fn no_executor_consumes_approval_records() {
    let (_where_, _how, evidence) =
        fail_verdict(task_59::run_scenario(&ctx_for(), "no-executor-verifies"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("approval consumers"),
        "evidence must list the consumer scan result:\n{joined}"
    );
    assert!(
        joined.contains("sweep_expired"),
        "evidence must name the only consumer (expiry):\n{joined}"
    );
    assert!(
        joined.contains("not an executor"),
        "evidence must classify the consumer honestly:\n{joined}"
    );
    let (_where2, _how2, evidence2) =
        fail_verdict(task_59::run_scenario(&ctx_for(), "replay-uncheckable"));
    let joined2 = evidence2.join("\n");
    assert!(
        joined2.contains("no rejection can be demonstrated"),
        "evidence must document the uncheckable replay:\n{joined2}"
    );
}
