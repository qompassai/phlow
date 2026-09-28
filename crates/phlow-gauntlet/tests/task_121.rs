//! Integration tests for task-121 (deny-all default + explicit opt-in,
//! diver Phase-2 Decision 1).
//!
//! Decision 1: diver ships `policy = { default = 'deny', rules = {} }`;
//! `policy_example.lua` (allow observe + local_reversible) ships BESIDE it,
//! never loaded implicitly — enabling it is one explicit line.
//!
//! The first three scenarios characterize today and pass: fresh `setup({})`
//! yields default-deny with no rules, `decide` fail-closes, and the setup
//! path never requires the example module implicitly (regression guard).
//! The fourth records the exact gap: `ai.harness.policy_example` does not
//! exist (`where = "policy-example-absent"`).
//! 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_121;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-121: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-121: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-121: HOME is not set"))
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
        "gauntlet-task-121-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-121: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the scenario to pass; panic on any failure so a regression in
/// the characterization is loud.
fn expect_pass(scenario: &str) -> Vec<String> {
    let ctx = ctx_for(scenario);
    match task_121::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-121 scenario '{scenario}' failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}

/// Expect the policy-example gap record; panic on any other outcome (a
/// pass or a driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_121::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "policy-example-absent",
                "task-121 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-121 scenario '{scenario}' passed: policy_example exists, contradicting the probed gap\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and a fresh `setup({})` yields
/// `default = 'deny'` with exactly `rules = {}` (init.lua setup,
/// policy.lua `M.new`).
#[test]
fn default_deny_all_no_rules() {
    assert_eq!(task_121::ID, "task-121");
    assert_eq!(task_121::NAME, "deny-all-default-opt-in");
    assert_eq!(task_121::KIND, TaskKind::NvimLua);
    let evidence = expect_pass("default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("default='deny'"),
        "evidence must show the deny default:\n{joined}"
    );
    assert!(
        joined.contains("rules is exactly {}"),
        "evidence must show no rules ship enabled:\n{joined}"
    );
}

/// V: `decide` fail-closes — empty rules deny, nil policy denies with
/// 'no policy configured', malformed requests and unknown risk classes
/// deny (policy.lua `M.decide`).
#[test]
fn decide_fail_closed_characterization() {
    let evidence = expect_pass("decide-fail-closed");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no policy configured"),
        "evidence must show the nil-policy denial:\n{joined}"
    );
    assert!(
        joined.contains("malformed request"),
        "evidence must show the malformed-request denial:\n{joined}"
    );
    assert!(
        joined.contains("unknown risk class"),
        "evidence must show the unknown-risk denial:\n{joined}"
    );
}

// --- adversarial ---

/// A: regression guard — the setup path never requires the example module
/// implicitly. Static source scan (init/supervisor/policy have no
/// `policy_example` reference) plus runtime checks (`package.loaded`
/// clean, post-setup rules exactly empty). A future "helpful" auto-enable
/// fails this test by construction.
#[test]
fn no_implicit_example_regression_guard() {
    let evidence = expect_pass("no-implicit-example");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no policy_example reference"),
        "evidence must show the static scan:\n{joined}"
    );
    assert!(
        joined.contains("package.loaded has no ai.harness.policy_example"),
        "evidence must show the runtime check:\n{joined}"
    );
}

/// A: the documented one-line opt-in has no target — `require` fails, so
/// the driver records the Decision-1 gap with the exact acceptance
/// criterion (module ships beside the default; the line allows observe,
/// still denies network).
#[test]
fn opt_in_absent_gap_recorded() {
    let (how, evidence) = expect_gap("opt-in-absent");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("policy_example") && joined.contains("not found"),
        "evidence must name the missing module:\n{joined}"
    );
    assert!(
        how.contains("Decision 1") || how.contains("decision-1"),
        "the 'how' must name Decision 1: {how}"
    );
    assert!(
        how.contains("observe"),
        "the 'how' must pin the allow-observe / deny-network acceptance: {how}"
    );
}
