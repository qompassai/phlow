//! Integration tests for task-80 (local model selection tradeoffs).
//!
//! The seam is REAL but does not meet the criteria: diver's
//! `ai.harness.adapter.negotiate(adapters, needs)` selects the FIRST
//! adapter in sorted name order whose probed boolean capabilities
//! satisfy every requested need — the vocabulary is exactly the
//! seven boolean CAPABILITY_KEYS, no model selection. Driver probes
//! against the REAL module confirm it (5 negotiate() probes, all
//! first-sorted-wins; a debug.getinfo-resolved source scan finds
//! zero tradeoff-vocabulary hits — the only `model` hits are
//! telemetry event-kind strings). The design's per-tool local model
//! selector (latency/memory/quality tradeoffs with explicit
//! rationale) has no implementation.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority —
//! whether diver should gain a model selector is Matt's call.
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

use phlow_gauntlet::tasks::task_80;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-80: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-80: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-80: HOME is not set"))
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
        "gauntlet-diver-rtp-80-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-80: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-80: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-80: cannot symlink ai: {e}"));
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
        "gauntlet-task-80-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-80: cannot build Ctx: {e}"));
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
            "task-80 passed: a model selector was invented, not found\\nevidence: {evidence:?}"
        ),
    }
}

/// Run a driver case; panic unless its own assertions hold.
fn driver_case(ctx: &Ctx, case: &'static str) -> task_80::CaseReport {
    let report = task_80::run_case(ctx, case)
        .unwrap_or_else(|e| panic!("task-80 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-80 case {case} must hold: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V: metadata contract pins the task; the driver probes the REAL
/// `ai.harness.adapter.negotiate` — 5 probes over 3 adapters with
/// distinct boolean capability profiles (plus driver-side quality
/// notes the module never reads) — and every probe selects the
/// first name in sorted order among the satisfying adapters. The
/// strategy is real, mechanical, and documented in the module's own
/// docstring. The task-level driver then combines all four cases
/// and reports the honest seam failure: the selector solves
/// boolean capability coverage, not model tradeoffs — the
/// model-selector question is Diver-owned and banked in the
/// task-level `how`, never fixed on gauntlet authority.
#[test]
fn selection_strategy_is_real_and_mechanical() {
    assert_eq!(task_80::ID, "task-80");
    assert_eq!(task_80::NAME, "local model selection tradeoffs");
    assert_eq!(task_80::KIND, TaskKind::NvimLua);
    assert_eq!(task_80::CASES.len(), 4, "2 validation + 2 adversarial");
    let ctx = ctx_for();
    let report = driver_case(&ctx, "selection_strategy_reality");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("first name in sorted order"),
        "evidence must show the strategy held on all 5 probes:\\n{joined}"
    );
    assert!(
        joined.contains("quality is not an input"),
        "evidence must show quality never decides:\\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` flags the Diver-owned finding without fixing it.
    let (where_, how, _) = fail_at_seam(task_80::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-80 must fail at the seam");
    assert!(
        how.contains("Diver-owned"),
        "the 'how' must flag the Diver-owned finding: {how}"
    );
    assert!(
        how.contains("no model registry"),
        "the 'how' must name the missing selector inputs: {how}"
    );
}

/// V: the model-selection scan — the driver resolves the LOADED
/// adapter.lua path via debug.getinfo (never a hardcoded path),
/// reads it plus sibling types.lua, and scans for model-selection
/// tradeoff vocabulary. The scan completes honestly; the raw hit
/// lists go to the harness probes.
#[test]
fn model_selection_scan_finds_no_tradeoff_vocabulary() {
    let ctx = ctx_for();
    let report = driver_case(&ctx, "model_selection_absent");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("tradeoff-vocabulary hits in adapter.lua + types.lua: 0"),
        "evidence must show zero tradeoff hits:\\n{joined}"
    );
    assert!(
        joined.contains("debug.getinfo"),
        "evidence must show the path was resolved, not hardcoded:\\n{joined}"
    );
}

// --- adversarial ---

/// A: the negotiation contract is boolean-only — over the driver's
/// machine-readable selection trace, every `needs` table holds only
/// booleans and every adapter's probed capabilities hold only the
/// boolean keys: no latency, memory, quality, or benchmark fields
/// anywhere in the contract.
#[test]
fn capability_needs_are_boolean_only() {
    let ctx = ctx_for();
    driver_case(&ctx, "selection_strategy_reality");
    let report = driver_case(&ctx, "capability_needs_boolean_only");
    assert_eq!(report.metrics["probes"], 5);
    assert_eq!(report.metrics["adapters"], 3);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no latency/memory/quality/benchmark fields"),
        "evidence must rule out tradeoff fields:\\n{joined}"
    );
}

/// A: no tradeoff record exists — the source scan finds zero
/// tradeoff-vocabulary hits (the only `model` hits are telemetry
/// event-kind strings), and the selection trace records selected
/// adapter names only: no rationale, no quality, no latency, no
/// memory per decision.
#[test]
fn no_tradeoff_record_exists() {
    let ctx = ctx_for();
    driver_case(&ctx, "selection_strategy_reality");
    driver_case(&ctx, "model_selection_absent");
    let report = driver_case(&ctx, "no_tradeoff_record");
    assert_eq!(report.metrics["tradeoff_hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("telemetry event-kind strings"),
        "evidence must classify the model hits:\\n{joined}"
    );
}
