//! Integration tests for task-115 (approval render attacks).
//!
//! Six driver scenarios — 2 validation, 2 adversarial (each test may
//! run multiple scenarios) — under headless Neovim. The approval
//! renderer is task-local and self-contained
//! (`lua/gauntlet/task_115.lua`); it does not use a diver seam (stated
//! honestly in the driver header). 50/50 split: 2 validation, 2
//! adversarial.
//!
//! The nvim binary comes from `GAUNTLET_NVIM_BIN`, with a fallback to
//! the known nightly path. Missing binaries panic: the gauntlet fails
//! closed, never skips.

use phlow_gauntlet::tasks::task_115;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required binary from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_bin(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-115: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-115: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-115: HOME is not set"))
}

/// Process-local sequence so concurrent `ctx_for` calls never collide.
static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build a `Ctx` for one scenario with its own scratch directory.
///
/// The driver is self-contained and uses no diver include path; the
/// crate's own `lua` directory is passed to satisfy `Ctx` (documented,
/// not a hidden dependency).
fn ctx_for(scenario: &str) -> Ctx {
    let nvim_bin = required_bin(
        "GAUNTLET_NVIM_BIN",
        &format!("{}/workspace/tools/neovim-nightly/bin/nvim", home_dir()),
    );
    let lua_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lua");
    let seq = WORKDIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let work_dir = std::env::temp_dir().join(format!(
        "gauntlet-task-115-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, lua_dir, work_dir)
        .unwrap_or_else(|e| panic!("task-115: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect a pass; panic on any failure or driver-harness error so a
/// breach can never masquerade as a finding.
fn expect_pass(scenario: &str) -> Vec<String> {
    let ctx = ctx_for(scenario);
    match task_115::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-115 scenario '{scenario}' failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}

/// V1: the clean render has zero escape bytes and binds SHA-256; the
/// escapes scenario strips ANSI sequences (0x1b count = 0).
#[test]
fn v_clean_and_escapes() {
    let ev = expect_pass("clean");
    assert!(
        ev.iter().any(|e| e.contains("0 escape bytes")),
        "no zero-escape evidence: {ev:?}"
    );
    assert!(
        ev.iter().any(|e| e.contains("SHA")),
        "no SHA-binding evidence: {ev:?}"
    );
    let ev = expect_pass("escapes");
    assert!(
        ev.iter().any(|e| e.contains("0x1b bytes in output = 0")),
        "escapes not stripped: {ev:?}"
    );
}

/// V2: elisions are visibly marked with the true hunk shown; urgency
/// text stays in the rationale block, out of UI chrome.
#[test]
fn v_elision_and_urgency() {
    let ev = expect_pass("elision");
    assert!(
        ev.iter().any(|e| e.contains("true hunk visible")),
        "hunk not visible: {ev:?}"
    );
    let ev = expect_pass("urgency");
    assert!(
        ev.iter().any(|e| e.contains("absent from chrome")),
        "urgency in chrome: {ev:?}"
    );
}

/// A1: rationale/edit mismatch — the behavioral summary states the
/// truth (DELETES) despite the lying "typo" rationale.
#[test]
fn a_rationale_edit_mismatch() {
    let ev = expect_pass("mismatch");
    assert!(
        ev.iter().any(|e| e.contains("DELETES")),
        "behavioral summary not honest: {ev:?}"
    );
}

/// A2: approval fatigue — 30 trivial + 1 consequential; the rate is
/// bounded (batches of ≤10), the consequential edit is isolated, and
/// there is no approve-all.
#[test]
fn a_approval_fatigue() {
    let ev = expect_pass("fatigue");
    assert!(
        ev.iter().any(|e| e.contains("no approve-all")),
        "approve-all not excluded: {ev:?}"
    );
    assert!(
        ev.iter().any(|e| e.contains("consequential edit isolated")),
        "consequential edit not isolated: {ev:?}"
    );
}
