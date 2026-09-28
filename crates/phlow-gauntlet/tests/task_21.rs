//! Integration tests for task-21 (saga compensating transactions).
//!
//! The seam is ABSENT: diver's `ai.harness` has a workflow naming layer
//! but no workflow runner or saga coordinator, so every driver scenario
//! reports `fail` with `where = "seam"`. The tests assert the evidence
//! documents the absence correctly rather than asserting a pass.
//! 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_21;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-21: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-21: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-21: HOME is not set"))
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
        "gauntlet-task-21-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-21: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// The seam verdict: fail with `where == "seam"`. Any other outcome means
/// the driver invented the seam the design forbids inventing.
fn seam_verdict(outcome: TaskOutcome, scenario: &str) -> (String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "seam",
                "task-21 scenario '{scenario}' failed somewhere else"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-21 scenario '{scenario}' passed: the seam was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and the default scenario reports
/// the seam absence with file-level evidence (naming-layer functions,
/// validating `types.lua`, `registry.lua`, `init.lua`).
#[test]
fn default_reports_seam_absence_with_source_evidence() {
    assert_eq!(task_21::ID, "task-21");
    assert_eq!(task_21::NAME, "saga compensating transactions");
    assert_eq!(task_21::KIND, TaskKind::NvimLua);
    let ctx = ctx_for("default");
    let (how, evidence) = seam_verdict(task_21::run_scenario(&ctx, "default"), "default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("register_workflow"),
        "evidence must name the naming-layer entry point:\n{joined}"
    );
    assert!(
        joined.contains("validate_run_spec"),
        "evidence must show spec.workflow is a validated label:\n{joined}"
    );
    assert!(
        how.contains("naming layer") && how.contains("no workflow runner"),
        "the 'how' must say naming layer present, runner absent: {how}"
    );
}

/// V: the task entry point (`run`, i.e. the default scenario) is honest
/// too — the absence finding is not an artifact of a non-default scenario.
#[test]
fn run_entry_point_also_reports_seam() {
    let ctx = ctx_for("default");
    let (how, evidence) = seam_verdict(task_21::run(&ctx), "run/default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("get_workflow"),
        "entry-point evidence must cover get_workflow:\n{joined}"
    );
    assert!(
        how.contains("design forbids inventing the coordinator"),
        "entry-point 'how' must cite the design constraint: {how}"
    );
}

// --- adversarial ---

/// A: the workflow registry is a naming layer, not an executor — an
/// attacker-style run through `harness.run` must ignore a registered
/// workflow's steps entirely. The scenario proves exactly one normal run
/// is created and the registry is never consulted.
#[test]
fn naming_layer_scenario_registry_is_never_consulted() {
    let ctx = ctx_for("naming-layer");
    let (how, evidence) = seam_verdict(task_21::run_scenario(&ctx, "naming-layer"), "naming-layer");
    assert!(
        how.contains("never consults the workflow registry"),
        "the 'how' must state the registry is ignored: {how}"
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("exactly one run exists"),
        "evidence must show the label created exactly one normal run:\n{joined}"
    );
    assert!(
        joined.contains("label-only confirmed"),
        "evidence must confirm the label executed nothing:\n{joined}"
    );
}

/// A: no executor callable exists anywhere on the harness surface — a
/// probe for `run_workflow`/`execute_workflow`/step-runner shapes must
/// come back empty rather than finding a hidden coordinator.
#[test]
fn no_executor_scenario_finds_no_hidden_coordinator() {
    let ctx = ctx_for("no-executor");
    let (_how, evidence) = seam_verdict(task_21::run_scenario(&ctx, "no-executor"), "no-executor");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("run_workflow"),
        "evidence must name the probed executor surface:\n{joined}"
    );
    assert!(
        joined.contains("absent") || joined.contains("no "),
        "evidence must state the executor is absent:\n{joined}"
    );
}
