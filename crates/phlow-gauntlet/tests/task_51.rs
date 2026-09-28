//! Integration tests for task-51 (byzantine worker detection).
//!
//! The seam is ABSENT: diver's fan-out consumers (`ai.a2a.fanout`,
//! `ai.a2a.orchestrator`) collect attributed per-worker results in
//! order, and the verifiers (`ai.harness.supervisor`,
//! `ai.harness.verdict`) grade single runs — no module computes a
//! verdict over multiple workers' answers, no quorum/voting rule
//! exists, and no dissenter is identified. Each test drives the
//! `task_51.lua` recon probe in headless Neovim against the REAL diver
//! Lua tree and asserts aspects of the honest `fail` verdict (`where =
//! "seam"`): 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_51;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-51: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-51: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-51: HOME is not set"))
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
        "gauntlet-diver-rtp-51-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-51: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-51: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-51: cannot symlink ai: {e}"));
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
        "gauntlet-task-51-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-51: cannot build Ctx: {e}"));
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
            "task-51 passed: aggregation machinery was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the fan-out-consumer probe
/// completes and reports the seam absence — `where = "seam"`, naming
/// the missing aggregation machinery.
#[test]
fn probe_reports_seam_absence() {
    assert_eq!(task_51::ID, "task-51");
    assert_eq!(task_51::NAME, "byzantine worker detection");
    assert_eq!(task_51::KIND, TaskKind::NvimLua);
    assert_eq!(task_51::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_verdict(task_51::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-51 must fail at the absent seam");
    let joined_ev = evidence.join("\n");
    assert!(
        joined_ev.contains("probed ai.a2a.fanout"),
        "the fan-out consumer module must actually load:\n{joined_ev}"
    );
    assert!(
        !joined_ev.contains("require failed"),
        "no module require may fail silently:\n{joined_ev}"
    );
    assert!(
        how.contains("no quorum/voting rule"),
        "the 'how' must name the absent aggregation: {how}"
    );
    assert!(
        how.contains("no dissenter is identified") || how.contains("dissenter"),
        "the 'how' must name the missing liar identification: {how}"
    );
}

/// V: the verifiers facet shows the per-run verifier and the run
/// lifecycle carry no cross-worker arbitration — `verdict.evaluate`
/// grades one run, the supervisor never compares workers' answers.
#[test]
fn verifiers_grade_single_runs_not_worker_quorums() {
    let (_where_, _how, evidence) = fail_verdict(task_51::run_scenario(&ctx_for(), "verifiers"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("ai.harness.verdict"),
        "evidence must show the verifier was probed:\n{joined}"
    );
    assert!(
        joined.contains("ai.harness.supervisor"),
        "evidence must show the supervisor was probed:\n{joined}"
    );
    assert!(
        joined.contains("ONE run") || joined.contains("one run"),
        "evidence must state verdict.evaluate is per-run:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_51::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: attribution exists but no verdict reads it — the probe documents
/// the per-worker attribution fields in the real result shapes (agent /
/// lang / state) AND the zero aggregation-API hits. Equivocation is
/// visible in principle and detected in practice by nothing; the
/// fail-closed facet confirms no aggregation machinery appeared.
#[test]
fn attribution_exists_but_no_verdict_reads_it() {
    let (_where_, _how, evidence) = fail_verdict(task_51::run_scenario(
        &ctx_for(),
        "attribution-without-verdict",
    ));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("attributed results EXIST") || joined.contains("attribution"),
        "evidence must document the per-worker attribution:\n{joined}"
    );
    assert!(
        joined.contains("equivocation"),
        "evidence must name the equivocation gap:\n{joined}"
    );
    let (_where2, _how2, evidence2) =
        fail_verdict(task_51::run_scenario(&ctx_for(), "fail-closed-recon"));
    let joined2 = evidence2.join("\n");
    assert!(
        joined2.contains("zero aggregation-API hits"),
        "fail-closed facet must record zero hits explicitly:\n{joined2}"
    );
}
