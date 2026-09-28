//! Integration tests for task-45 (plugin dependency confusion).
//!
//! The defense the design requires is ABSENT at an EXISTING seam:
//! diver's `ai.harness.registry.register_builtins` loads the six
//! built-in adapters via path-ordered `pcall(require,
//! 'ai.harness.adapters.' .. name)` with no trusted-source pinning.
//! The probe demonstrates the confusion LIVE: in a child nvim process
//! whose runtimepath is the clean default with the probe-owned shadow
//! prepended and the diver root appended, the shadow `acp` (execution marker only) wins the
//! require — its code EXECUTES for the trusted name `acp` and
//! `get_adapter('acp')` returns the shadow. The registry's
//! duplicate-registration rejection does not help: the confusion
//! happens at load time, before registration. Each test drives the
//! `task_45.lua` probe in headless Neovim against the REAL diver Lua
//! tree and asserts aspects of the honest `fail` verdict
//! (`where = "resolution"`): 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.
//!
//! Diver-owned security finding: flagged in the probe evidence, never
//! fixed on gauntlet authority.

use phlow_gauntlet::tasks::task_45;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-45: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-45: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-45: HOME is not set"))
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
        "gauntlet-task-45-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-45: cannot build Ctx: {e}"));
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
            "task-45 passed: trusted-first adapter resolution was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the probe completes and reports
/// the resolution failure — `where = "resolution"` (not "seam": the
/// seam exists, the defense is what's absent), naming the
/// path-ordered require with no trusted-source pinning.
#[test]
fn probe_reports_resolution_failure() {
    assert_eq!(task_45::ID, "task-45");
    assert_eq!(task_45::NAME, "plugin dependency confusion");
    assert_eq!(task_45::KIND, TaskKind::NvimLua);
    let (where_, how, _evidence) = fail_verdict(task_45::run(&ctx_for()));
    assert_eq!(
        where_, "resolution",
        "task-45 must fail at resolution — the seam exists, the defense does not"
    );
    assert!(
        how.contains("no trusted-source pinning"),
        "the 'how' must name the missing pinning: {how}"
    );
}

/// V: the probe exercised the real registry — the legit `acp`
/// registers and resolves, duplicate registration is rejected, and
/// the `registry.lua` source was scanned for pin/trust verification
/// (zero hits) before the shadow was planted.
#[test]
fn probe_exercises_the_real_registry() {
    let (_where_, _how, evidence) = fail_verdict(task_45::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("duplicate registration rejected: adapter already registered: acp"),
        "evidence must show the real duplicate rejection ran:\n{joined}"
    );
    assert!(
        joined.contains("pin/trust tokens in registry.lua: 0"),
        "evidence must show the pin-token scan ran with zero hits:\n{joined}"
    );
}

// --- adversarial ---

/// A: the shadow executes live — the child probe's marker file was
/// written (the shadow's code ran at require time for the trusted
/// name `acp`) and `get_adapter('acp')` returned the shadow, not the
/// legitimate adapter.
#[test]
fn shadow_executes_live() {
    let (_where_, _how, evidence) = fail_verdict(task_45::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("shadow execution marker written: true"),
        "evidence must show the shadow's code executed:\n{joined}"
    );
    assert!(
        joined.contains("get_adapter(acp) returns the SHADOW: true"),
        "evidence must show the shadow won resolution:\n{joined}"
    );
}

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// resolution finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_45::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}
