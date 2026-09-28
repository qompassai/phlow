//! Integration tests for task-41 (authority attenuation).
//!
//! The seam is ABSENT as designed: diver's `ai.harness.supervisor` has
//! real delegation — `spawn_child` sets `parent_id` and calls
//! `create`, so a parent → child → grandchild chain links correctly —
//! but run tables carry no tool grant, no authority set, and no
//! privilege list, and `spawn_child` performs no grant-intersection
//! step. Policy is supervisor-global (`supervisor.new` `opts.policy`),
//! never per-run. The design's "child authority ⊆ parent authority",
//! transitive attenuation to depth 2, and the denial naming the
//! missing grant have no seam to attach to. Each test drives the
//! `task_41.lua` probe in headless Neovim against the REAL diver Lua
//! tree and asserts aspects of the honest `fail` verdict
//! (`where = "seam"`): 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.
//!
//! Diver-owned finding: flagged in the probe evidence, never fixed on
//! gauntlet authority.

use phlow_gauntlet::tasks::task_41;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-41: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-41: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-41: HOME is not set"))
}

/// Process-local sequence so concurrent `ctx_for` calls never collide.
static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build a `Ctx` with its own scratch directory. The workdir is unique
/// per call (pid + a process-local counter): tests running in parallel
/// get disjoint directories.
fn ctx_for() -> Ctx {
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
        "gauntlet-task-41-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-41: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(180);
    ctx
}

/// Unwrap the expected `fail` verdict, or panic with the details.
fn fail_verdict(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-41 passed: per-run authority attenuation was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the probe completes and reports
/// the seam absence — `where = "seam"`, naming the missing
/// grant-intersection at delegation.
#[test]
fn probe_reports_seam_absence() {
    assert_eq!(task_41::ID, "task-41");
    assert_eq!(task_41::NAME, "authority attenuation");
    assert_eq!(task_41::KIND, TaskKind::NvimLua);
    let (where_, how, _evidence) = fail_verdict(task_41::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-41 must fail at the absent seam");
    assert!(
        how.contains("no per-run authority"),
        "the 'how' must name the missing per-run authority: {how}"
    );
}

/// V: the probe exercised the real delegation path — a parent →
/// child → grandchild chain with `parent_id` linked at each depth —
/// before concluding that authority is absent.
#[test]
fn probe_exercises_the_real_delegation_path() {
    let (_where_, _how, evidence) = fail_verdict(task_41::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("parent -> child -> grandchild, parent_id linked"),
        "evidence must show the three-depth delegation chain ran:\n{joined}"
    );
    assert!(
        joined.contains("spawn_child sets spec.parent_id and calls M.create"),
        "evidence must show the spawn_child → create path was traced:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_41::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: no run table carries an authority field and no intersection step
/// exists — the run keys show no tools grant / grant set / privilege
/// list, and policy is proven supervisor-global, so "child authority
/// ⊆ parent authority" and its transitivity are unexpressible.
#[test]
fn no_authority_fields_and_no_intersection_step() {
    let (_where_, how, evidence) = fail_verdict(task_41::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no run carries an authority field"),
        "evidence must show the run tables carry no authority:\n{joined}"
    );
    assert!(
        joined.contains("supervisor-global collaborator"),
        "evidence must show policy is supervisor-global, never per-run:\n{joined}"
    );
    assert!(
        how.contains("no intersection step"),
        "the 'how' must name the missing intersection step: {how}"
    );
}
