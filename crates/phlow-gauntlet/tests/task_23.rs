//! Integration tests for task-23 (bounded dynamic fan-out).
//!
//! Each test drives the `task_23.lua` Neovim driver for one scenario and
//! asserts the verdict. The driver exercises the REAL spawn path
//! (`supervisor.spawn_child` → `supervisor.create`) against the real
//! supervisor bound (`RUNS_MAX = 256`, `run_count` never decrements).
//! 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_23;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-23: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-23: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-23: HOME is not set"))
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
        "gauntlet-task-23-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-23: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(180);
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
        } => panic!("task-23 scenario '{scenario}' failed at '{where_}': {how}\n{evidence:?}"),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the small-N fan-out spawns 5
/// children, starts them, completes them, and finishes the parent last.
#[test]
fn small_n_fan_out_spawns_and_completes() {
    assert_eq!(task_23::ID, "task-23");
    assert_eq!(task_23::NAME, "bounded dynamic fan-out");
    assert_eq!(task_23::KIND, TaskKind::NvimLua);
    let ctx = ctx_for("default");
    let evidence = pass_evidence(task_23::run_scenario(&ctx, "default"), "default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("spawned children: 5"),
        "expected 5 spawned children:\n{joined}"
    );
    assert!(
        joined.contains("parent finished after its children"),
        "expected the parent to finish last:\n{joined}"
    );
}

/// V: structured ownership — every child carries the parent's `parent_id`
/// and `root_id`, and the parent's `children` list links all 5.
#[test]
fn children_carry_parent_and_root_ids() {
    let ctx = ctx_for("default");
    let evidence = pass_evidence(task_23::run_scenario(&ctx, "default"), "default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("parent.children linked: 5 (structured ownership)"),
        "expected the parent children list to link all 5:\n{joined}"
    );
    assert!(
        joined.contains("parent_id + root_id propagated to every child"),
        "expected parent_id/root_id propagation:\n{joined}"
    );
}

// --- adversarial ---

/// A: attacker-controlled N=10000 spawn attempts against the real spawn
/// path. Exactly 255 succeed (the parent holds one of the 256 run slots);
/// every over-bound attempt fails with the explicit
/// 'supervisor run bound exceeded' error — no hang, no unbounded growth.
#[test]
fn attacker_n_10000_hits_the_live_bound() {
    let ctx = ctx_for("attacker-n");
    let evidence = pass_evidence(task_23::run_scenario(&ctx, "attacker-n"), "attacker-n");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("spawn attempts: 10000"),
        "expected 10000 attempts:\n{joined}"
    );
    assert!(
        joined.contains("spawned ok: 255; rejected: 9745"),
        "expected exactly 255 successes and 9745 rejections:\n{joined}"
    );
    assert!(
        joined.contains(
            "every over-bound spawn failed with the explicit error 'supervisor run bound exceeded'"
        ),
        "every rejection must carry the explicit bound error:\n{joined}"
    );
    assert!(
        joined.contains("run_count pinned at the cap"),
        "run_count must stay pinned: no unbounded growth:\n{joined}"
    );
}

/// A: fork-bomb shape — every run spawns 3 children until the supervisor
/// refuses. The cascade must terminate exactly at the total-run cap with
/// explicit refusals, and the driver must document the real mechanism:
/// there is NO explicit depth bound, only the total-run cap.
#[test]
fn recursive_cascade_terminates_at_the_cap() {
    let ctx = ctx_for("recursive");
    let evidence = pass_evidence(task_23::run_scenario(&ctx, "recursive"), "recursive");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("cascade terminated exactly at the total-run cap: no fork bomb"),
        "the cascade must stop at the cap:\n{joined}"
    );
    assert!(
        joined.contains("every refusal was the explicit 'supervisor run bound exceeded' error"),
        "every refusal must carry the explicit bound error:\n{joined}"
    );
    assert!(
        joined.contains("no explicit depth bound exists"),
        "the driver must document the missing depth bound honestly:\n{joined}"
    );
}
