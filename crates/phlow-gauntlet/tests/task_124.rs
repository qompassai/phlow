//! Integration tests for task-124 (prompt fallback and abort atomicity,
//! diver Phase-2 Decision 2).
//!
//! The design: missing pieces fall back to `vim.ui.select` (adapter, from
//! the registry) / `vim.ui.input` (workflow, goal), in that order;
//! aborting any prompt aborts the WHOLE command — zero runs created, zero
//! events appended. A whitespace-only goal is treated as abort, not as a
//! goal.
//!
//! The command module does not exist today, so every scenario records that
//! exact gap (`where = "command-module-absent"`) with the full acceptance
//! contract. The `whitespace-goal` scenario additionally characterizes the
//! building block: `validate_run_spec` ACCEPTS `'   '`, so the trim rule
//! must live in the command layer.
//! 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_124;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-124: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-124: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-124: HOME is not set"))
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
        "gauntlet-task-124-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-124: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the command-module gap record; panic on any other outcome (a
/// pass or a driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_124::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "command-module-absent",
                "task-124 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-124 scenario '{scenario}' passed: :HarnessRun exists, contradicting the probed gap\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and bare `:HarnessRun` does not
/// exist — the gap record pins the prompt order (adapter -> workflow ->
/// goal), one run, one `run.created` event.
#[test]
fn bare_harnessrun_gap() {
    assert_eq!(task_124::ID, "task-124");
    assert_eq!(task_124::NAME, "prompt-fallback-abort-atomicity");
    assert_eq!(task_124::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("bare-harnessrun");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("exists(\":HarnessRun\") == 0"),
        "evidence must show the command is absent:\n{joined}"
    );
    assert!(
        how.contains("adapter") && how.contains("workflow") && how.contains("goal"),
        "the 'how' must pin the prompt order: {how}"
    );
    assert!(
        how.contains("run.created"),
        "the 'how' must pin the single run.created event: {how}"
    );
}

/// V: `:HarnessRun a2a` does not exist either — the gap record pins that
/// partial args prompt only for the missing pieces.
#[test]
fn partial_args_gap() {
    let (how, _evidence) = expect_gap("partial-args");
    assert!(
        how.contains("missing"),
        "the 'how' must pin prompting only for missing pieces: {how}"
    );
}

// --- adversarial ---

/// A: aborting any of the three prompts (nil OR empty string — identical)
/// must abort the whole command: zero runs, zero events. Unverifiable
/// today; the gap record pins the atomicity contract.
#[test]
fn abort_atomicity_gap() {
    let (how, _evidence) = expect_gap("abort-atomicity");
    assert!(
        how.contains("zero runs") && how.contains("zero events"),
        "the 'how' must pin the all-or-nothing contract: {how}"
    );
    assert!(
        how.contains("nil") && how.contains("empty"),
        "the 'how' must pin nil/empty equivalence: {how}"
    );
}

/// A: a whitespace-only goal is an abort, not a goal — and the driver
/// characterizes WHY the trim rule must live in the command layer:
/// `validate_run_spec` accepts `'   '`.
#[test]
fn whitespace_goal_gap() {
    let (how, evidence) = expect_gap("whitespace-goal");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("ACCEPTS goal='   '"),
        "evidence must show validate_run_spec accepts whitespace-only goals:\n{joined}"
    );
    assert!(
        how.contains("trim"),
        "the 'how' must pin the command-layer trim rule: {how}"
    );
}
