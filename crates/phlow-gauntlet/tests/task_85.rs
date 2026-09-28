//! Integration tests for task-85 (provider usage accounting integrity).
//!
//! The seam is REAL but does not meet the criteria: diver's
//! `ai.harness.budget` is the real per-run budget ledger, but it
//! has NO usage-ingestion layer. Driver probes against the REAL
//! module confirm it: well-formed usage is exact (consume(100,
//! 'token') -> snapshot.used.token == 100); `consume` takes
//! (budget, kind, amount) — resolved from the real budget.lua source via
//! debug.getinfo('S'), no flag parameter — and the snapshot carries bare
//! numbers (estimated vs measured indistinguishable); consume(b, 'token',
//! 1e12) succeeds with no cap and no flag — the absurd value enters
//! the ledger at face value; and the module exposes exactly
//! new/check/consume/remaining/exhausted/snapshot — no correction
//! API, no audit trail, so a provider correction is possible only by
//! raw table mutation, which keeps no max-observed and writes no
//! audit note.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority —
//! whether the ledger should gain usage-ingestion discipline
//! (estimated flags, sanity caps, append-only corrections) is Matt's
//! call.
//!
//! Four cases — 2 validation (driver probes), 2 adversarial
//! (harness probes over the driver's machine-readable traces) —
//! each self-checking; the driver then reports the honest seam
//! failure.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`,
//! with fallbacks to Matt's known tool paths. Missing binaries or
//! directories panic with a clear message: the gauntlet fails
//! closed, never skips.

use phlow_gauntlet::tasks::task_85;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-85: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-85: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-85: HOME is not set"))
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
        "gauntlet-diver-rtp-85-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-85: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-85: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-85: cannot symlink ai: {e}"));
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
        "gauntlet-task-85-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-85: cannot build Ctx: {e}"));
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
            "task-85 passed: a usage-ingestion layer was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

/// Run a driver case; panic unless its own assertions hold.
fn driver_case(ctx: &Ctx, case: &'static str) -> task_85::CaseReport {
    let report = task_85::run_case(ctx, case)
        .unwrap_or_else(|e| panic!("task-85 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-85 case {case} must hold: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V: metadata contract pins the task; the driver probes the REAL
/// `ai.harness.budget` — well-formed usage is recorded exactly
/// (consume(100,'token') -> snapshot.used.token == 100). The
/// task-level driver then combines all four cases and reports the
/// honest seam failure: the ledger is exact for well-formed usage
/// but has no usage-ingestion discipline — the ingestion question is
/// Diver-owned and flagged in the task-level `how`, never fixed on
/// gauntlet authority.
#[test]
fn wellformed_usage_is_exact() {
    assert_eq!(task_85::ID, "task-85");
    assert_eq!(task_85::NAME, "provider usage accounting integrity");
    assert_eq!(task_85::KIND, TaskKind::NvimLua);
    assert_eq!(task_85::CASES.len(), 4, "2 validation + 2 adversarial");
    let ctx = ctx_for();
    let report = driver_case(&ctx, "wellformed_usage_exact");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("snapshot.used.token = 100"),
        "evidence must show the exact ledger entry:\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` flags the Diver-owned finding without fixing it.
    let (where_, how, _) = fail_at_seam(task_85::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-85 must fail at the seam");
    assert!(
        how.contains("Diver-owned"),
        "the 'how' must flag the Diver-owned finding: {how}"
    );
    assert!(
        how.contains("no usage-ingestion layer"),
        "the 'how' must name the missing layer: {how}"
    );
}

/// V: the estimated/measured distinction does not exist — the driver
/// resolves `budget.consume`'s declaration from the real budget.lua
/// source via debug.getinfo('S') (budget, kind, amount — no flag
/// parameter) and confirms the snapshot carries bare numbers with no
/// estimated flag.
#[test]
fn estimated_flag_is_absent() {
    let ctx = ctx_for();
    let report = driver_case(&ctx, "estimated_flag_absent");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("params=3"),
        "evidence must show consume takes exactly 3 params:\n{joined}"
    );
    assert!(
        joined.contains("flag param=false"),
        "evidence must show consume has no flag parameter:\n{joined}"
    );
    assert!(
        joined.contains("estimated flag: false"),
        "evidence must show the snapshot has no estimated flag:\n{joined}"
    );
}

// --- adversarial ---

/// A: absurd usage is absorbed silently — over the driver's
/// machine-readable usage trace, consume(b,'token',1e12) returned
/// ok, the snapshot shows used.token == 1e12, and the trace records
/// flagged=false, capped=false: budget enforcement downstream eats
/// the absurd number as-is.
#[test]
fn absurd_usage_absorbed_silently() {
    let ctx = ctx_for();
    let report = driver_case(&ctx, "absurd_usage_silent");
    assert_eq!(report.metrics["used_token"], 1e12);
    assert_eq!(report.metrics["flagged"], false);
    assert_eq!(report.metrics["capped"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("at face value"),
        "evidence must show the absurd value entered the ledger:\n{joined}"
    );
}

/// A: shrinking has no correction path — over the driver's trace,
/// the module exposes exactly
/// new/check/consume/remaining/exhausted/snapshot (no correction, no
/// audit; correction_api=false, audit_trail=false): a provider
/// correction is possible only by raw table mutation, which keeps no
/// max-observed and writes no audit note.
#[test]
fn shrinking_has_no_correction_path() {
    let ctx = ctx_for();
    let report = driver_case(&ctx, "shrinking_has_no_correction");
    assert_eq!(report.metrics["correction_api"], false);
    assert_eq!(report.metrics["audit_trail"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("raw table mutation"),
        "evidence must name the only downward path:\n{joined}"
    );
}
