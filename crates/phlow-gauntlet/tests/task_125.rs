//! Integration tests for task-125 (run selection TOCTOU for cancel/resume,
//! diver Phase-2 Decision 2).
//!
//! The design: `:HarnessCancel` / `:HarnessResume` with no arg offer
//! `vim.ui.select` over eligible runs (live runs for cancel;
//! terminal-but-not-completed for resume). The TOCTOU core: a run that goes
//! terminal between listing and acting must make the act fail cleanly
//! (`'run is already terminal'`), with no corruption and no error event.
//! Selection is by run id, never by workflow name.
//!
//! The picker commands do not exist today, so those scenarios record the
//! exact gap (`where = "command-module-absent"`); the TOCTOU core and the
//! unknown-run path exercise `supervisor.cancel` directly and pass.
//! 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_125;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-125: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-125: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-125: HOME is not set"))
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
        "gauntlet-task-125-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-125: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the scenario to pass; panic on any failure so a regression in
/// the characterization is loud.
fn expect_pass(scenario: &str) -> Vec<String> {
    let ctx = ctx_for(scenario);
    match task_125::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-125 scenario '{scenario}' failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}

/// Expect the command-module gap record; panic on any other outcome (a
/// pass or a driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_125::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "command-module-absent",
                "task-125 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-125 scenario '{scenario}' passed: the picker command exists, contradicting the probed gap\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and `:HarnessCancel` does not
/// exist — the gap record pins the live-run picker contract (selection by
/// run id, identical workflow names disambiguated).
#[test]
fn cancel_picker_gap() {
    assert_eq!(task_125::ID, "task-125");
    assert_eq!(task_125::NAME, "run-selection-toctou");
    assert_eq!(task_125::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("cancel-picker-missing");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("exists(\":HarnessCancel\") == 0"),
        "evidence must show the command is absent:\n{joined}"
    );
    assert!(
        how.contains("LIVE") || how.contains("live"),
        "the 'how' must pin the live-run picker contract: {how}"
    );
    assert!(
        how.contains("run id") || how.contains("RUN ID"),
        "the 'how' must pin selection by run id: {how}"
    );
}

/// V: `:HarnessResume` does not exist either — the gap record pins the
/// resumable-run picker contract (failed/cancelled/timed_out/interrupted;
/// never completed, never running).
#[test]
fn resume_picker_gap() {
    let (how, _evidence) = expect_gap("resume-picker-missing");
    assert!(
        how.contains("failed") && how.contains("cancelled"),
        "the 'how' must pin the resumable set: {how}"
    );
    assert!(
        how.contains("never completed"),
        "the 'how' must pin the completed/running exclusion: {how}"
    );
}

// --- adversarial ---

/// A: the TOCTOU core — the run goes terminal between "listing" and
/// "acting"; `supervisor.cancel` returns `'run is already terminal'`
/// cleanly: run state unchanged, no new sink events.
#[test]
fn cancel_after_terminal_clean_refusal() {
    let evidence = expect_pass("cancel-after-terminal");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("already terminal"),
        "evidence must show the clean refusal:\n{joined}"
    );
    assert!(
        joined.contains("no corruption"),
        "evidence must show the run state is unchanged:\n{joined}"
    );
    assert!(
        joined.contains("unchanged by the refused cancel"),
        "evidence must show no events were appended:\n{joined}"
    );
}

/// A: `supervisor.cancel` of a never-existing id returns a clean
/// `'unknown run'` error — no panic, no state touched.
#[test]
fn cancel_unknown_run_clean_error() {
    let evidence = expect_pass("cancel-unknown-run");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("unknown run"),
        "evidence must show the clean error:\n{joined}"
    );
    assert!(
        joined.contains("unchanged"),
        "evidence must show no state was touched:\n{joined}"
    );
}
