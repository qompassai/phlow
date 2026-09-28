//! Integration tests for task-17 (nvim edit-check-fix loop).
//!
//! Each test drives the `task_17.lua` Neovim driver for one scenario and
//! asserts the verdict. 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths, and `GAUNTLET_LUAC_BIN` (else the
//! Lua 5.4.8 toolchain luac). Missing binaries or directories panic with
//! a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_17;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-17: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-17: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

/// Resolve a required file from env or fallback. Panics (fail closed) when
/// missing.
fn required_file(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-17: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.is_file() {
        panic!(
            "task-17: required file does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-17: HOME is not set"))
}

/// Build a `Ctx` for one scenario with its own scratch directory.
fn ctx_for(scenario: &str) -> Ctx {
    let nvim_bin = required_dir(
        "GAUNTLET_NVIM_BIN",
        &format!("{}/workspace/tools/neovim-nightly/bin/nvim", home_dir()),
    );
    let diver_lua = required_dir(
        "GAUNTLET_DIVER_LUA",
        &format!("{}/workspace/repos/diver/lua", home_dir()),
    );
    let work_dir = std::env::temp_dir().join(format!("gauntlet-task-17-{scenario}"));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-17: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Resolve the luac binary the driver must use. The path is forwarded
/// explicitly (not via mutated process environment) so tests cannot race.
fn luac_bin() -> PathBuf {
    required_file(
        "GAUNTLET_LUAC_BIN",
        &format!("{}/workspace/tools/lua-5.4.8/src/luac", home_dir()),
    )
}

/// Run one driver scenario with the pinned luac binary.
fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    task_17::run_scenario_with_luac(ctx, scenario, &luac_bin())
}

/// The pinned luac path, for golden-transcript assertions.
fn luac_bin_str() -> String {
    luac_bin().to_string_lossy().into_owned()
}

/// Unwrap a passing verdict into its evidence, or panic with the failure.
fn pass_evidence(outcome: TaskOutcome, scenario: &str) -> Vec<String> {
    match outcome {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-17 scenario '{scenario}' failed at '{where_}': {how}\n{evidence:?}"),
    }
}

/// Count transcript lines starting with `prefix`.
fn count_prefix(evidence: &[String], prefix: &str) -> usize {
    evidence
        .iter()
        .filter(|line| line.starts_with(prefix))
        .count()
}

// --- validation ---

/// V: the loop completes — edit, check surfaces the error, fix, re-verify
/// clean. Golden assertion on the exact operator-visible transcript: the
/// operator and the report see the same lines, in the same order.
#[test]
fn loop_completes_edit_check_fix_verify() {
    let ctx = ctx_for("default");
    let evidence = pass_evidence(run_scenario(&ctx, "default"), "default");
    let bin = luac_bin_str();
    let expected = vec![
        format!("[task-17] luac-bin={bin}"),
        "[task-17] scenario=default".to_string(),
        "[task-17] stage=edit file=target.lua lines=5".to_string(),
        "[task-17] stage=check attempt=1 cmd=\"luac -p target.lua\"".to_string(),
        "[task-17] luac-error file=target.lua line=6 msg=')' expected (to close '(' at line 5) near <eof>"
            .to_string(),
        "[task-17] hint: fix the error above, then re-run luac".to_string(),
        "[task-17] stage=fix attempt=1 file=target.lua lines=5".to_string(),
        "[task-17] stage=check attempt=2 cmd=\"luac -p target.lua\"".to_string(),
        "[task-17] luac-clean file=target.lua".to_string(),
        "[task-17] result: pass checks=2 fixes=1".to_string(),
    ];
    assert_eq!(
        evidence, expected,
        "operator transcript drifted from the golden sequence"
    );
}

/// V: the luac error is surfaced legibly — the operator sees the file, the
/// line, the message, and what to do next; progress renders sanely with
/// each stage exactly once.
#[test]
fn error_surfaced_legibly_for_operator() {
    let ctx = ctx_for("default");
    let evidence = pass_evidence(run_scenario(&ctx, "default"), "default");

    // Stages render in order, each exactly once: no duplicated or missing
    // steps in what the operator watched.
    let stages: Vec<&str> = evidence
        .iter()
        .filter_map(|line| line.strip_prefix("[task-17] stage="))
        .map(|rest| rest.split(' ').next().unwrap_or(""))
        .collect();
    assert_eq!(
        stages,
        ["edit", "check", "fix", "check"],
        "stage sequence is not edit -> check -> fix -> check: {stages:?}"
    );

    // The error line names the file, the line, and luac's message.
    let error_line = evidence
        .iter()
        .find(|line| line.contains("luac-error"))
        .expect("no luac-error line in transcript");
    assert!(
        error_line.contains("file=target.lua"),
        "error does not name the file: {error_line}"
    );
    assert!(
        error_line.contains("line=6"),
        "error does not name the line: {error_line}"
    );
    assert!(
        error_line.contains("msg=')' expected"),
        "error does not carry luac's message: {error_line}"
    );

    // What-to-do-next guidance immediately follows the error.
    let error_idx = evidence
        .iter()
        .position(|l| l.contains("luac-error"))
        .unwrap();
    assert!(
        evidence[error_idx + 1].contains("hint: fix the error above, then re-run luac"),
        "no next-step hint after the error: {:?}",
        evidence[error_idx + 1]
    );

    // The loop ends visibly clean.
    assert!(
        evidence
            .iter()
            .any(|l| l.contains("luac-clean file=target.lua")),
        "no luac-clean line: {evidence:?}"
    );
    assert!(
        evidence.iter().any(|l| l.contains("result: pass")),
        "no result line: {evidence:?}"
    );
}

// --- adversarial ---

/// A: the first fix is itself broken (a *different* error). The loop must
/// catch the new error on re-check — it must never trust a fix blindly —
/// and then converge with a second fix.
#[test]
fn bad_fix_caught_and_recovered() {
    let ctx = ctx_for("bad-fix");
    let evidence = pass_evidence(run_scenario(&ctx, "bad-fix"), "bad-fix");
    let joined = evidence.join("\n");

    // Both errors appear: the original and the one the bad fix introduced.
    // The second error proves the loop re-checked instead of assuming.
    assert!(
        joined.contains("msg=')' expected (to close '(' at line 5) near <eof>"),
        "original error missing: {joined}"
    );
    assert!(
        joined.contains("msg=unexpected symbol near '+'"),
        "the bad fix's *new* error was not caught on re-check: {joined}"
    );
    assert_eq!(
        count_prefix(&evidence, "[task-17] luac-error"),
        2,
        "expected exactly two surfaced errors: {joined}"
    );
    assert_eq!(
        count_prefix(&evidence, "[task-17] stage=fix"),
        2,
        "expected two fix attempts: {joined}"
    );
    assert!(
        joined.contains("result: pass checks=3 fixes=2"),
        "loop did not converge after the bad fix: {joined}"
    );
}

/// A: fixes that never converge (the error oscillates) must stop after a
/// bounded number of attempts and fail honestly — no infinite fix loop,
/// no fabricated success.
#[test]
fn nonconverging_loop_fails_bounded() {
    let ctx = ctx_for("no-converge");
    match run_scenario(&ctx, "no-converge") {
        TaskOutcome::Pass { evidence } => {
            panic!("non-converging fix loop reported pass: {evidence:?}")
        }
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(where_, "fix", "wrong failure stage: {where_}");
            assert!(
                how.contains("did not converge after 3 attempts"),
                "failure does not name the bound: {how}"
            );
            // The operator sees exactly where the loop gave up: the last
            // luac error with file, line, and message.
            assert!(
                how.contains("target.lua:6: unexpected symbol near '+'"),
                "failure does not carry the last luac error: {how}"
            );
            // Bounded: exactly 3 fixes, 4 checks — the loop stopped.
            assert_eq!(
                count_prefix(&evidence, "[task-17] stage=fix"),
                3,
                "fix attempts were not bounded at 3: {evidence:?}"
            );
            assert_eq!(
                count_prefix(&evidence, "[task-17] stage=check"),
                4,
                "expected 4 checks: {evidence:?}"
            );
            // Oscillation is visible in the transcript: the two errors
            // alternate, which is why no fix could ever converge.
            let joined = evidence.join("\n");
            assert!(
                joined.contains("msg=')' expected") && joined.contains("msg=unexpected symbol"),
                "transcript does not show the oscillation: {joined}"
            );
        }
    }
}

/// Task metadata (ID/NAME/KIND) is intact.
#[test]
fn task_metadata_intact() {
    assert_eq!(task_17::ID, "task-17");
    assert_eq!(task_17::NAME, "nvim edit-check-fix loop");
    assert!(matches!(task_17::KIND, TaskKind::NvimLua));
}
