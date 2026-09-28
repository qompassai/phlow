//! Integration tests for task-64 (deterministic tool selection).
//!
//! The seam is PRESENT: `ai.harness.adapter.negotiate` — "Choose the
//! first adapter (in sorted name order) whose probed capabilities satisfy
//! every requested need" — is capability-based selection with a
//! documented precedence rule. Each test drives the `task_64.lua` probe
//! in headless Neovim against the REAL diver Lua tree (`negotiate` is
//! the real function; only the adapters are mocks) and asserts the
//! `pass` verdict: 2 validation, 2 adversarial.
//!
//! Two caveats are banked for Matt (diver-owned, never fixed on gauntlet
//! authority): C1 — no rationale record is produced or logged per
//! selection (the rule is documented, not logged); C2 — precedence is
//! hardcoded sorted-name order, not a precedence config.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_64;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-64: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-64: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-64: HOME is not set"))
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
        "gauntlet-diver-rtp-64-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-64: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-64: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-64: cannot symlink ai: {e}"));
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
        "gauntlet-task-64-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-64: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(180);
    ctx
}

/// Unwrap the expected `pass` verdict, or panic with the details.
fn pass_verdict(outcome: TaskOutcome) -> Vec<String> {
    match outcome {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-64 failed at '{where_}': {how}\nevidence: {evidence:?}"),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the single-match facet drives the
/// REAL `negotiate` and the only satisfying adapter is selected.
#[test]
fn single_match_selects_the_satisfier() {
    assert_eq!(task_64::ID, "task-64");
    assert_eq!(task_64::NAME, "deterministic tool selection");
    assert_eq!(task_64::KIND, TaskKind::NvimLua);
    assert_eq!(task_64::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let evidence = pass_verdict(task_64::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("winner=beta"),
        "evidence must show the single satisfier won:\n{joined}"
    );
}

/// V: the ambiguity facet — two adapters satisfy, registered in reverse
/// name order, and the first in SORTED name order wins: the documented
/// precedence rule decides, independent of registration order.
#[test]
fn ambiguity_resolves_by_sorted_name() {
    let evidence = pass_verdict(task_64::run_scenario(&ctx_for(), "ambiguity-sorted-name"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("winner=alpha despite zeta being registered first"),
        "evidence must show sorted-name precedence beat registration order:\n{joined}"
    );
    assert!(
        joined.contains("first adapter (in sorted name order)"),
        "evidence must cite the documented precedence rule:\n{joined}"
    );
    assert!(
        joined.contains("CAVEAT C2 (banked)"),
        "the pass must bank the precedence-config caveat:\n{joined}"
    );
}

// --- adversarial ---

/// A: the 100-run stability battery — 100 ambiguous selections with
/// alternating insertion order choose the same winner every time: no
/// hash-order or timing dependence. This is the design's determinism
/// proof.
#[test]
fn hundred_run_stability() {
    let evidence = pass_verdict(task_64::run_scenario(&ctx_for(), "hundred-run-stability"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("100/100 runs chose alpha"),
        "evidence must show all 100 runs agreed:\n{joined}"
    );
}

/// A: the probe-flap boundary — an intermittently failing probe changes
/// the outcome, but that is an INPUT change (probe outcomes are inputs
/// to the pure function), not nondeterminism: probe failure becomes
/// "unavailable", never raises. The facet also surfaces caveat C1 (no
/// rationale logging), which stays banked.
#[test]
fn probe_flap_is_an_input_change() {
    let evidence = pass_verdict(task_64::run_scenario(&ctx_for(), "probe-flap-boundary"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("probe failure -> adapter skipped (never raises)"),
        "evidence must show the flap is an input change, not nondeterminism:\n{joined}"
    );
    assert!(
        joined.contains("caveat C1"),
        "evidence must surface the rationale-logging gap:\n{joined}"
    );
    assert!(
        joined.contains("CAVEAT C1 (banked)"),
        "the pass must bank the rationale-logging caveat:\n{joined}"
    );
}
