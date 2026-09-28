//! Integration tests for task-95 (approval render integrity / WYSIWYG).
//!
//! The lua seam is ABSENT as designed: diver's `ai.harness.approval` is
//! a data-only queue exposing no render API; the approval record
//! carries a free-text `summary`, never diff bytes. Its header says
//! "The single approval surface renders from this queue", but no
//! render function exists in lua/ai — the surface lives outside the
//! lua tree. No diff-render pipeline, no escape neutralization, no
//! elision marking, no semantic-change summary exists in the lua
//! layer.
//!
//! Each test drives the `task_95.lua` probe in headless Neovim against
//! the REAL `ai.harness` modules and asserts the honest `fail` at
//! `"seam"`: 2 validation, 2 adversarial.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`,
//! with fallbacks to Matt's known tool paths. Missing binaries or
//! directories panic with a clear message: the gauntlet fails closed,
//! never skips.

use phlow_gauntlet::tasks::task_95;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-95: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-95: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-95: HOME is not set"))
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
        "gauntlet-diver-rtp-95-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-95: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-95: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-95: cannot symlink ai: {e}"));
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
        "gauntlet-task-95-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-95: cannot build Ctx: {e}"));
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
            "task-95 passed: an approval render UI was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: the approval queue is data-only — no render API — and the
/// approval record carries no diff bytes. The queue's header promises
/// "the single approval surface renders from this queue", but no
/// render function exists in lua/ai.
#[test]
fn no_render_api() {
    assert_eq!(task_95::ID, "task-95");
    assert_eq!(task_95::NAME, "approval render integrity (WYSIWYG)");
    assert_eq!(task_95::KIND, TaskKind::NvimLua);
    assert_eq!(task_95::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_95::run_case(&ctx_for(), "no_render_api")
        .unwrap_or_else(|e| panic!("task-95 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-render case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["render_api"], false);
    assert_eq!(report.metrics["record_has_diff"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("render_api=Some(false)"),
        "evidence must show the absent render API:\n{joined}"
    );
}

/// V2: no diff-render pipeline exists in the approval path anywhere in
/// lua/ai — the token scan for diff_render/render_diff/approval_ui
/// finds zero hits. The task-level driver then fails at the seam.
#[test]
fn no_diff_render_pipeline() {
    let report = task_95::run_case(&ctx_for(), "no_diff_render_pipeline")
        .unwrap_or_else(|e| panic!("task-95 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-pipeline case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["render_hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("render_hits=Some(0)"),
        "evidence must show the zero-hit scan:\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass).
    let (where_, how, _) = fail_at_seam(task_95::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-95 must fail at the seam");
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

/// A1 (harness probe): no escape-neutralization exists — the token
/// scan for strip_escapes/neutralize/sanitize_render finds zero hits.
/// With no renderer, an ANSI-bomb in a diff is out of scope: there is
/// no render surface to harden.
#[test]
fn no_escape_neutralization() {
    let report = task_95::run_case(&ctx_for(), "no_escape_neutralization")
        .unwrap_or_else(|e| panic!("task-95 case failed to run: {e}"));
    assert!(
        report.passed,
        "escapes case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["escape_hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("escape_hits=Some(0)"),
        "evidence must show the zero-hit scan:\n{joined}"
    );
}

/// A2 (harness probe): no elision marking exists — the token scan for
/// elision/collapsed_hunk/hunk_marker finds zero hits. Nothing
/// collapses hunks, so no marker discipline can be verified — and the
/// approval record carries no diff bytes for a WYSIWYG check to bind.
#[test]
fn no_elision_marking() {
    let report = task_95::run_case(&ctx_for(), "no_elision_marking")
        .unwrap_or_else(|e| panic!("task-95 case failed to run: {e}"));
    assert!(
        report.passed,
        "elision case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["elision_hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("elision_hits=Some(0)"),
        "evidence must show the zero-hit scan:\n{joined}"
    );
}
