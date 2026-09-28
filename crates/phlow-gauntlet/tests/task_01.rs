//! Integration tests for task-01: fan-out/fan-in verdict aggregation.
//!
//! 50/50 split: 2 validation (default fan-out reaches terminal states;
//! all-succeed aggregates pass) + 2 adversarial (adapter start() raises yet
//! the run still terminates and counts; mid-run cancel yields cancelled and
//! aggregate fail). Missing nvim/diver paths panic with a clear message:
//! fail closed, never skip.

use phlow_gauntlet::tasks::task_01;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;
use std::time::Duration;

/// Resolve the headless Neovim binary. Panics with a clear message when
/// missing: the gauntlet fails closed, never skips.
fn nvim_bin() -> PathBuf {
    let home = match std::env::var("HOME") {
        Ok(home) => home,
        Err(_) => panic!("gauntlet test: HOME is not set; cannot locate the nvim fallback"),
    };
    let fallback = PathBuf::from(home).join("workspace/tools/neovim-nightly/bin/nvim");
    let path = std::env::var("GAUNTLET_NVIM_BIN")
        .map(PathBuf::from)
        .unwrap_or(fallback);
    if !path.is_file() {
        panic!(
            "gauntlet test: nvim binary missing: {} (set GAUNTLET_NVIM_BIN)",
            path.display()
        );
    }
    path
}

/// Resolve diver's `lua/` directory. Panics with a clear message when
/// missing: the gauntlet fails closed, never skips.
fn diver_lua_dir() -> PathBuf {
    let home = match std::env::var("HOME") {
        Ok(home) => home,
        Err(_) => panic!("gauntlet test: HOME is not set; cannot locate the diver fallback"),
    };
    let fallback = PathBuf::from(home).join("workspace/repos/diver/lua");
    let path = std::env::var("GAUNTLET_DIVER_LUA")
        .map(PathBuf::from)
        .unwrap_or(fallback);
    if !path.is_dir() {
        panic!(
            "gauntlet test: diver lua dir missing: {} (set GAUNTLET_DIVER_LUA)",
            path.display()
        );
    }
    path
}

/// Build a `Ctx` for one test: disjoint scratch dir per test name so tests
/// stay independent under parallel execution.
fn test_ctx(test_name: &str) -> Ctx {
    let work_dir = std::env::temp_dir().join(format!("gauntlet-task-01-{test_name}"));
    let mut ctx = Ctx::new(nvim_bin(), diver_lua_dir(), work_dir)
        .expect("gauntlet test: Ctx::new rejected non-empty paths");
    ctx.timeout = Duration::from_secs(60);
    ctx
}

/// The driver's evidence lines on a Pass outcome; panics (fail closed) when
/// the driver reports failure so the where/how is visible, not swallowed.
fn pass_evidence(scenario: &str, outcome: TaskOutcome) -> Vec<String> {
    match outcome {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-01 scenario '{scenario}': driver reported fail at '{where_}': {how}\nevidence:\n{}",
            evidence.join("\n")
        ),
    }
}

fn has_line(evidence: &[String], needle: &str) -> bool {
    evidence.iter().any(|line| line.contains(needle))
}

/// V: default fan-out leaves no run behind — 5 terminal states observed.
#[test]
fn default_fanout_all_runs_terminal() {
    let ctx = test_ctx("default-fanout");
    let evidence = pass_evidence("default", task_01::run_scenario(&ctx, "default"));
    assert!(
        has_line(&evidence, "terminal=5/5"),
        "expected 5/5 terminal runs, evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        has_line(&evidence, "finished_events=5"),
        "expected 5 run.finished events in the sink, evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        has_line(&evidence, "lost=0"),
        "expected no lost runs, evidence:\n{}",
        evidence.join("\n")
    );
}

/// V: when all 5 workers succeed, the aggregate verdict is pass.
#[test]
fn all_succeed_aggregates_pass() {
    let ctx = test_ctx("all-succeed");
    let evidence = pass_evidence("all-succeed", task_01::run_scenario(&ctx, "all-succeed"));
    assert!(
        has_line(&evidence, "terminal=5/5"),
        "expected 5/5 terminal runs, evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        has_line(&evidence, "completed=5"),
        "expected 5 completed runs, evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        has_line(&evidence, "aggregate=pass"),
        "expected aggregate=pass, evidence:\n{}",
        evidence.join("\n")
    );
}

/// A: an adapter whose start() raises must still terminate as failed and be
/// counted — not hang in 'queued' and not vanish from the fan-in.
#[test]
fn adapter_start_raises_still_terminates_and_counts() {
    let ctx = test_ctx("adapter-raises");
    let evidence = pass_evidence(
        "adapter-raises",
        task_01::run_scenario(&ctx, "adapter-raises"),
    );
    assert!(
        has_line(&evidence, "terminal=5/5"),
        "raised run must still reach terminal, evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        has_line(&evidence, "finished_events=5"),
        "raised run must emit run.finished, evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        has_line(&evidence, "raised=yes"),
        "raised run must be marked in evidence, evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        has_line(&evidence, "aggregate=fail"),
        "4/5 completed must aggregate to fail, evidence:\n{}",
        evidence.join("\n")
    );
}

/// A: cancelling a run mid-fan-out yields a cancelled run and aggregate fail.
#[test]
fn cancel_during_fanout_yields_cancelled_and_aggregate_fail() {
    let ctx = test_ctx("cancel-mid-run");
    let evidence = pass_evidence("default", task_01::run_scenario(&ctx, "default"));
    assert!(
        has_line(&evidence, "cancelled=1"),
        "expected exactly 1 cancelled run, evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        has_line(&evidence, "terminal=5/5"),
        "expected 5/5 terminal runs, evidence:\n{}",
        evidence.join("\n")
    );
    assert!(
        has_line(&evidence, "aggregate=fail"),
        "3 completed + 1 failed + 1 cancelled must aggregate to fail, evidence:\n{}",
        evidence.join("\n")
    );
}
