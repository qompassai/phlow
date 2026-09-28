//! Integration tests for task-18 (worker crash recovery).
//!
//! Each test drives the `task_18.lua` Neovim driver — against diver's REAL
//! `ai.harness.supervisor` — for one scenario and asserts the verdict.
//! 50/50 split: 2 validation, 2 adversarial. The first validation test is
//! the CLI golden-output test: it runs the `gauntlet` binary itself and
//! asserts the operator-facing report (the JSON the operator reads)
//! carries an unambiguous verdict and a legible cause chain.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_18;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-18: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-18: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-18: HOME is not set"))
}

fn nvim_bin() -> PathBuf {
    required_dir(
        "GAUNTLET_NVIM_BIN",
        &format!("{}/workspace/tools/neovim-nightly/bin/nvim", home_dir()),
    )
}

fn diver_lua() -> PathBuf {
    required_dir(
        "GAUNTLET_DIVER_LUA",
        &format!("{}/workspace/repos/diver/lua", home_dir()),
    )
}

/// Build a `Ctx` for one scenario with its own scratch directory.
fn ctx_for(scenario: &str) -> Ctx {
    let work_dir = std::env::temp_dir().join(format!("gauntlet-task-18-{scenario}"));
    let mut ctx = Ctx::new(nvim_bin(), diver_lua(), work_dir).expect("task-18: cannot build Ctx");
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Unwrap a passing verdict into its evidence, or panic with the failure.
fn pass_evidence(outcome: TaskOutcome, scenario: &str) -> Vec<String> {
    match outcome {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-18 scenario '{scenario}' failed at '{where_}': {how}\n{evidence:?}"),
    }
}

/// The `gauntlet` CLI binary under test, provided by Cargo to integration
/// tests of this package.
fn gauntlet_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gauntlet"))
}

// --- validation ---

/// V: CLI golden output — `gauntlet run task-18` prints one JSON report
/// whose evidence shows the operator an unambiguous verdict: `failed`
/// after 4/4 attempts, with every crash cause in the chain and the
/// ceiling note. This is what the operator actually reads; the test pins
/// its shape and its legibility.
#[test]
fn cli_report_shows_unambiguous_failed_verdict_with_cause_chain() {
    assert_eq!(task_18::ID, "task-18");
    assert_eq!(task_18::NAME, "worker crash recovery");
    assert_eq!(task_18::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let work_dir = std::env::temp_dir().join("gauntlet-task-18-cli-golden");
    let output = std::process::Command::new(gauntlet_bin())
        .arg("run")
        .arg("task-18")
        .arg("--nvim-bin")
        .arg(nvim_bin())
        .arg("--diver-lua")
        .arg(diver_lua())
        .arg("--work-dir")
        .arg(&work_dir)
        .output()
        .expect("task-18: cannot run gauntlet CLI");
    assert!(
        output.status.success(),
        "gauntlet run task-18 exited nonzero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("task-18: CLI output is not JSON");
    assert_eq!(report["id"], "task-18");
    assert_eq!(report["kind"], "nvim-lua");
    assert_eq!(report["outcome"], "pass");
    let evidence: Vec<String> = report["evidence"]
        .as_array()
        .expect("evidence is an array")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let view = evidence
        .iter()
        .find(|line| line.starts_with("operator view:"))
        .expect("expected an operator-view line in the CLI report");
    // The verdict is unambiguous: failed, attempts fully accounted for.
    assert!(
        view.contains("verdict=failed") && view.contains("attempts=4/4"),
        "operator view must read failed 4/4, got: {view}"
    );
    // The cause chain is legible: every attempt's crash plus the give-up.
    for cause in [
        "signal 11 (SIGSEGV)",
        "signal 9 (SIGKILL, OOM)",
        "heartbeat lost",
        "code 1 (adapter init failed)",
    ] {
        assert!(
            view.contains(cause),
            "operator view lost a crash cause '{cause}': {view}"
        );
    }
    assert!(
        view.contains("retry attempt ceiling exceeded"),
        "operator view must name the give-up reason: {view}"
    );
    // The operator can tell this is not a timeout or a cancellation.
    assert!(
        !view.contains("timed_out") && !view.contains("cancelled"),
        "failed verdict must not read as timed_out/cancelled: {view}"
    );
}

/// V: transient crashes recover — two crashes, retries, then the worker
/// succeeds on attempt 3 and the run completes (no failure verdict).
#[test]
fn transient_crash_recovers_and_completes() {
    let ctx = ctx_for("transient-crash");
    let evidence = pass_evidence(
        task_18::run_scenario(&ctx, "transient-crash"),
        "transient-crash",
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("run completed on attempt 3 after two crash recoveries"),
        "expected completion on attempt 3, got:\n{joined}"
    );
    assert!(
        joined.contains("verdict=completed attempts=3/4 (recovered, no failure)"),
        "expected a clean recovered operator view, got:\n{joined}"
    );
}

// --- adversarial ---

/// A: a duplicate crash report for the same attempt is refused — the
/// transition `retry_wait -> retry_wait` is illegal, the attempt counter
/// does not move, and the refusal is operator-visible as a diagnostic.
/// The legitimate retry still promotes and completes.
#[test]
fn duplicate_crash_report_spends_no_retry_budget() {
    let ctx = ctx_for("duplicate-crash");
    let evidence = pass_evidence(
        task_18::run_scenario(&ctx, "duplicate-crash"),
        "duplicate-crash",
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains(
            "duplicate crash report refused: invalid transition retry_wait -> retry_wait"
        ),
        "expected the duplicate to be refused on the transition table, got:\n{joined}"
    );
    assert!(
        joined.contains("attempt counter still 2: the duplicate did not spend budget"),
        "expected the attempt counter pinned at 2, got:\n{joined}"
    );
    assert!(
        joined.contains("invalid_transition diagnostic recorded: the refusal is operator-visible"),
        "expected an operator-visible diagnostic, got:\n{joined}"
    );
    assert!(
        joined.contains("legitimate retry promoted and completed on attempt 2"),
        "expected recovery to still work after the duplicate, got:\n{joined}"
    );
}

/// A: a storm of 10 crash injections cannot escape the bound — attempts
/// stay capped at 4, 7 reports hit the ceiling, the run finishes `failed`
/// exactly once, and a late crash after the terminal finish changes
/// nothing (no resurrection, no double finish).
#[test]
fn crash_storm_cannot_escape_the_retry_bound() {
    let ctx = ctx_for("crash-storm");
    let evidence = pass_evidence(task_18::run_scenario(&ctx, "crash-storm"), "crash-storm");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("10 crash injections: 7 hit the ceiling, attempt capped at 4"),
        "expected 7 ceiling hits with attempt capped at 4, got:\n{joined}"
    );
    assert!(
        joined.contains("late crash after give-up refused: retry attempt ceiling exceeded"),
        "expected the late crash to be refused, got:\n{joined}"
    );
    assert!(
        joined
            .contains("run.finished fired exactly once: the finish is idempotent under the storm"),
        "expected exactly one run.finished, got:\n{joined}"
    );
    let view = evidence
        .iter()
        .find(|line| line.starts_with("operator view:"))
        .expect("expected an operator-view line");
    assert!(
        view.contains("verdict=failed") && view.contains("attempts=4/4"),
        "storm operator view must read failed 4/4, got: {view}"
    );
}
