//! Integration tests for task-37 (confused deputy).
//!
//! The seam is ABSENT as designed: diver's `ai.harness.policy` decides
//! only the IMMEDIATE tool request — `AiHarnessToolRequest` carries no
//! requester-chain field, `rule_matches` consults only
//! risk/tools/paths/endpoints, and an extra provenance field is silently
//! ignored. A deputy-caused invocation of the privileged tool decides
//! byte-identically to a direct, properly-approved invocation, and
//! two-hop laundering (A→C→B) is equally invisible. The design's
//! "denial names the broken chain" is impossible: there is no chain to
//! name. Each test drives the `task_37.lua` probe in headless Neovim
//! against the REAL diver Lua tree and asserts aspects of the honest
//! `fail` verdict (`where = "seam"`): 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.
//!
//! Diver-owned finding: flagged in the probe evidence, never fixed on
//! gauntlet authority.

use phlow_gauntlet::tasks::task_37;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-37: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-37: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-37: HOME is not set"))
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
        "gauntlet-task-37-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-37: cannot build Ctx: {e}"));
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
            "task-37 passed: requester-chain awareness was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the probe completes and reports
/// the seam absence — `where = "seam"`, naming the missing
/// requester-chain concept in the authorization check.
#[test]
fn probe_reports_seam_absence() {
    assert_eq!(task_37::ID, "task-37");
    assert_eq!(task_37::NAME, "confused deputy");
    assert_eq!(task_37::KIND, TaskKind::NvimLua);
    let (where_, how, _evidence) = fail_verdict(task_37::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-37 must fail at the absent seam");
    assert!(
        how.contains("chain"),
        "the 'how' must name the missing requester chain: {how}"
    );
}

/// V: the probe exercised the real authorization check — the mock tools
/// were registered in a real policy, and the direct invocation of the
/// privileged tool decided 'approval' — before concluding.
#[test]
fn probe_exercises_the_real_policy_check() {
    let (_where_, _how, evidence) = fail_verdict(task_37::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("direct invocation of vault.export")
            && joined.contains("decision=approval"),
        "evidence must show the default (direct, approved) scenario ran:\n{joined}"
    );
    assert!(
        joined.contains("notes.read") && joined.contains("cache.write"),
        "evidence must show the mock tools A and C were registered:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_37::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: the deputy-caused invocation is INDISTINGUISHABLE from the direct
/// one — byte-identical decisions for the caused and the two-hop
/// laundered invocations — so the design's chain-aware denial has no
/// seam: the extra provenance fields are silently ignored by the check.
#[test]
fn deputy_caused_invocation_is_indistinguishable() {
    let (_where_, how, evidence) = fail_verdict(task_37::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("byte-identical to the direct decision: true"),
        "evidence must show the deputy-caused decision matched the direct one:\n{joined}"
    );
    assert!(
        joined.contains("two-hop laundering is equally invisible"),
        "evidence must show the A->C->B laundering scenario ran and matched:\n{joined}"
    );
    assert!(
        joined.contains("no chain, principal, delegated_by, on_behalf_of"),
        "evidence must state why the design's denial is impossible:\n{joined}"
    );
    assert!(
        how.contains("decides identically"),
        "the 'how' must state the indistinguishability: {how}"
    );
}
