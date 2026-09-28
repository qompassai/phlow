//! Integration tests for task-123 (`:HarnessRun` argument parsing contract,
//! diver Phase-2 Decision 2).
//!
//! The design: hybrid — explicit args when given, prompts for the rest;
//! everything after `--` is the goal VERBATIM.
//!
//! The command module does not exist today, so the parsing scenarios
//! record that exact gap (`where = "command-module-absent"`) and pin the
//! contract; `goal-required` characterizes the building block behind
//! "never creates a goal-less run" (`types.validate_run_spec` requires a
//! non-empty goal) and passes.
//! 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_123;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-123: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-123: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-123: HOME is not set"))
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
        "gauntlet-task-123-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-123: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the scenario to pass; panic on any failure so a regression in
/// the characterization is loud.
fn expect_pass(scenario: &str) -> Vec<String> {
    let ctx = ctx_for(scenario);
    match task_123::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-123 scenario '{scenario}' failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}

/// Expect the command-module gap record; panic on any other outcome (a
/// pass or a driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_123::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "command-module-absent",
                "task-123 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-123 scenario '{scenario}' passed: :HarnessRun exists, contradicting the probed gap\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and `:HarnessRun` does not exist
/// (`exists() == 0`, E492) — the gap record pins the hybrid parsing
/// contract (adapter, workflow, verbatim goal after `--`).
#[test]
fn parse_harnessrun_gap() {
    assert_eq!(task_123::ID, "task-123");
    assert_eq!(task_123::NAME, "harnessrun-arg-parsing");
    assert_eq!(task_123::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("parse-harnessrun");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("exists(\":HarnessRun\") == 0"),
        "evidence must show the command is absent:\n{joined}"
    );
    assert!(
        how.contains("VERBATIM") || how.contains("verbatim"),
        "the 'how' must pin the verbatim-goal contract: {how}"
    );
}

/// V: `validate_run_spec` requires a non-empty goal — the building block
/// behind "never creates a goal-less run" (types.lua). Passes today.
#[test]
fn goal_required_by_validate_run_spec() {
    let evidence = expect_pass("goal-required");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("non-empty goal"),
        "evidence must show the goal requirement:\n{joined}"
    );
    assert!(
        joined.contains("rejects missing goal") && joined.contains("rejects empty goal"),
        "evidence must show both rejections:\n{joined}"
    );
}

// --- adversarial ---

/// A: a goal containing `--` — only the first `--` is the separator, so
/// `:HarnessRun a2a w -- -- -- --` must yield goal `-- -- --`. Unverifiable
/// today; the gap record pins the acceptance.
#[test]
fn double_dash_separator_gap() {
    let (how, _evidence) = expect_gap("double-dash-in-goal");
    assert!(
        how.contains("FIRST") || how.contains("first"),
        "the 'how' must pin the first-`--`-is-the-separator rule: {how}"
    );
}

/// A: `%`/`#` must pass through literal (never filename-expanded) and a
/// newline in the goal must be preserved or cleanly rejected, never
/// silently truncated. Unverifiable today; the gap record pins it.
#[test]
fn percent_hash_newline_gap() {
    let (how, _evidence) = expect_gap("percent-hash-newline");
    assert!(
        how.contains("filename-expanded") || how.contains("LITERAL"),
        "the 'how' must pin the literal `%`/`#` rule: {how}"
    );
    assert!(
        how.contains("truncated"),
        "the 'how' must pin the never-truncate rule: {how}"
    );
}
