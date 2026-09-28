//! Integration tests for task-122 (capability-based risk escalation,
//! diver Phase-2 Decision 1, Fix 3 detail).
//!
//! The design: launch classifies risk from the adapter's pcall'd `probe()`
//! capabilities — `remote = true` -> `'network'`, else `'process'` — and
//! consults `policy.decide` before starting the adapter. Broken probes fall
//! back to `'network'` (fail-closed).
//!
//! Today launch builds no policy request at all (supervisor.lua line 182),
//! so every scenario records that precise gap
//! (`where = "no-launch-policy-request"`) with scenario-specific evidence:
//! a probe-call spy and a decide-call spy around a real launch both read
//! zero. The records are the Phase-2 acceptance artifact.
//! 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_122;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-122: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-122: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-122: HOME is not set"))
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
        "gauntlet-task-122-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-122: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the no-classification gap record; panic on any other outcome (a
/// pass or a driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_122::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "no-launch-policy-request",
                "task-122 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-122 scenario '{scenario}' passed: launch classified something, contradicting the probed gap\nevidence: {evidence:?}"
        ),
    }
}

/// Shared assertions for the gap record: both spies read zero and the
/// 'how' names the launch path.
fn assert_no_classification(how: &str, evidence: &[String], scenario: &str) {
    let joined = evidence.join("\n");
    assert!(
        joined.contains("probe() calls during launch: 0"),
        "task-122 '{scenario}': evidence must show launch never probed:\n{joined}"
    );
    assert!(
        joined.contains("decide calls during launch: 0"),
        "task-122 '{scenario}': evidence must show decide was never consulted:\n{joined}"
    );
    assert!(
        how.contains("supervisor.lua") || how.contains("launch"),
        "task-122 '{scenario}': the 'how' must name the launch path: {how}"
    );
}

// --- validation ---

/// V: an adapter attesting `remote = true` launches with no `'network'`
/// policy request built — metadata contract pins the task first.
#[test]
fn remote_true_builds_no_network_request() {
    assert_eq!(task_122::ID, "task-122");
    assert_eq!(task_122::NAME, "capability-risk-escalation");
    assert_eq!(task_122::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("remote-true");
    assert_no_classification(&how, &evidence, "remote-true");
    assert!(
        how.contains("'network'"),
        "the 'how' must pin the 'network' acceptance: {how}"
    );
}

/// V: an adapter attesting `remote = false` launches with no `'process'`
/// policy request built.
#[test]
fn remote_false_builds_no_process_request() {
    let (how, evidence) = expect_gap("remote-false");
    assert_no_classification(&how, &evidence, "remote-false");
    assert!(
        how.contains("'process'"),
        "the 'how' must pin the 'process' acceptance: {how}"
    );
}

// --- adversarial ---

/// A: a raising `probe()` is never even called by launch — there is no
/// pcall and no fail-closed `'network'` fallback.
#[test]
fn probe_raises_no_failclosed_fallback() {
    let (how, evidence) = expect_gap("probe-raises");
    assert_no_classification(&how, &evidence, "probe-raises");
    assert!(
        how.contains("pcall") && how.contains("fail-closed"),
        "the 'how' must pin the pcall/fail-closed acceptance: {how}"
    );
}

/// A: malformed capability shapes are rejected by the strict probe
/// contract (`remote = 'yes'`, non-table caps, `remote = nil` — all
/// "must be a boolean" / "must return a table"; a probeless adapter is
/// rejected at registration), yet launch classifies nothing either way.
#[test]
fn malformed_probe_no_classification() {
    let (how, evidence) = expect_gap("malformed-probe");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("must be a boolean"),
        "evidence must show the strict-boolean rejection:\n{joined}"
    );
    assert!(
        joined.contains("must return a table"),
        "evidence must show the non-table rejection:\n{joined}"
    );
    assert!(
        joined.contains("register_adapter rejects probeless adapter"),
        "evidence must show the probeless registration rejection:\n{joined}"
    );
    assert_no_classification(&how, &evidence, "malformed-probe");
}
