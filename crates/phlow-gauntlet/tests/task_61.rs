//! Integration tests for task-61 (plan schema validation).
//!
//! The seam is ABSENT: diver has no machine plan representation and no
//! plan-schema validator. `ai.rose`'s `M.plan` emits plan TEXT for
//! humans ("Output ONLY the plan as plain text"); the harness executes
//! runs, not plans; the registry's `register_workflow` accepts arbitrary
//! def shapes (only `adapter` is validated). Each test drives the
//! `task_61.lua` probe in headless Neovim against the REAL diver Lua
//! tree and asserts aspects of the honest `fail` verdict (`where =
//! "seam"`): 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_61;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-61: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-61: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-61: HOME is not set"))
}

/// Process-local sequence so concurrent `ctx_for` calls never collide.
static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build a scratch diver runtime-path shim: the Lua drivers append
/// `DIVER_LUA_DIR` to the rtp and `require('ai....')`, which needs
/// `<dir>/lua/ai/...` on the rtp. The shim is two symlinks —
/// `lua -> <diver-lua>` and `ai -> <diver-lua>/ai` — in a scratch dir;
/// no diver file is touched. The real tree comes from
/// `GAUNTLET_DIVER_LUA` (or Matt's known checkout).
fn diver_rtp_shim(seq: u64) -> PathBuf {
    let real = required_path(
        "GAUNTLET_DIVER_LUA",
        &format!("{}/workspace/repos/diver/lua", home_dir()),
    );
    let shim = std::env::temp_dir().join(format!(
        "gauntlet-diver-rtp-61-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-61: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-61: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-61: cannot symlink ai: {e}"));
    }
    shim
}

/// Build a `Ctx` with its own scratch directory and diver rtp shim.
/// The workdir is unique per call (pid + a process-local counter):
/// tests running in parallel get disjoint directories.
fn ctx_for() -> Ctx {
    let nvim_bin = required_path(
        "GAUNTLET_NVIM_BIN",
        &format!("{}/workspace/tools/neovim-nightly/bin/nvim", home_dir()),
    );
    let seq = WORKDIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let diver_lua = diver_rtp_shim(seq);
    let work_dir = std::env::temp_dir().join(format!(
        "gauntlet-task-61-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-61: cannot build Ctx: {e}"));
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
        TaskOutcome::Pass { evidence } => {
            panic!("task-61 passed: plan machinery was invented, not found\nevidence: {evidence:?}")
        }
    }
}

// --- validation ---

/// V: metadata contract pins the task; the plan-schema scan completes
/// and reports the absent seam — `where = "seam"` — with the `how`
/// naming the missing plan type and the unmeetable structural criterion.
#[test]
fn probe_reports_absent_seam() {
    assert_eq!(task_61::ID, "task-61");
    assert_eq!(task_61::NAME, "plan schema validation");
    assert_eq!(task_61::KIND, TaskKind::NvimLua);
    assert_eq!(task_61::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_verdict(task_61::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-61 must fail at the absent seam");
    let joined_ev = evidence.join("\n");
    assert!(
        joined_ev.contains("no machine plan type"),
        "evidence must state there is no plan type:\n{joined_ev}"
    );
    assert!(
        how.contains("no plan type"),
        "the 'how' must name the unmeetable structural criterion: {how}"
    );
}

/// V: the workflow-def facet — the closest "plan" object accepts an
/// arbitrary shape; only `adapter` is validated, so no step/tool/
/// dependency schema exists to reject a malformed plan against.
#[test]
fn workflow_def_has_no_plan_schema() {
    let (where_, _how, evidence) = fail_verdict(task_61::run_scenario(
        &ctx_for(),
        "workflow-def-has-no-plan-schema",
    ));
    assert_eq!(
        where_, "seam",
        "an accepted malformed shape must not flip the verdict to 'recon'"
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("ACCEPTED a malformed plan shape"),
        "evidence must show the malformed shape was accepted:\n{joined}"
    );
    assert!(
        joined.contains("round-trips verbatim"),
        "evidence must show the def is stored, not validated:\n{joined}"
    );
}

// --- adversarial ---

/// A: the malformed-plan facet — a mock malformed plan (missing step,
/// unknown tool, cyclic deps) has no validator to be fed to: every
/// plausible validator entry point is nil, so the design's "rejected
/// with the schema violation named" is unmeetable.
#[test]
fn malformed_plan_cannot_be_rejected() {
    let (where_, _how, evidence) = fail_verdict(task_61::run_scenario(
        &ctx_for(),
        "malformed-plan-cannot-be-rejected",
    ));
    assert_eq!(where_, "seam", "no validator must mean the seam finding");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("validator entry points probed: 5; present: 0"),
        "evidence must enumerate the nil validator entry points:\n{joined}"
    );
}

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding. Also asserts the rose planner-role control sample is
/// classified UNRELATED, never counted as plan machinery.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, evidence) = fail_verdict(task_61::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("classified UNRELATED"),
        "the rose planner-role control sample must be classified, not counted:\n{joined}"
    );
}
