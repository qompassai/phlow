//! Integration tests for task-69 (hallucinated tool rejection).
//!
//! The seam is PRESENT: `ai.rose.tools.M.call(name, args)` — "Call a
//! tool by name; never raises, always returns a status table." Dispatch
//! is an exact table lookup `by_name[name]` with
//! `assert(spec, 'unknown tool: ' .. tostring(name))` BEFORE
//! `validate_args(args, spec)`: name resolution precedes arg parsing by
//! construction, and the module contains no fuzzy matching, no
//! edit-distance, no did-you-mean. Each test drives the `task_69.lua`
//! probe in headless Neovim against the REAL diver dispatcher and
//! asserts the `pass` verdict: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_69;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-69: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-69: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-69: HOME is not set"))
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
        "gauntlet-diver-rtp-69-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-69: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-69: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-69: cannot symlink ai: {e}"));
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
        "gauntlet-task-69-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-69: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(180);
    ctx
}

/// Unwrap the expected `pass` verdict, or panic with the details.
fn pass_verdict(outcome: TaskOutcome) -> Vec<String> {
    match outcome {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-69 failed at '{where_}': {how}\nevidence: {evidence:?}"),
    }
}

// --- validation ---

/// V: metadata contract pins the task; a real tool (`file_read`)
/// dispatches by exact name and returns the exact bytes.
#[test]
fn real_tool_dispatches_by_exact_name() {
    assert_eq!(task_69::ID, "task-69");
    assert_eq!(task_69::NAME, "hallucinated tool rejection");
    assert_eq!(task_69::KIND, TaskKind::NvimLua);
    assert_eq!(task_69::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let evidence = pass_verdict(task_69::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("file_read dispatched by exact name"),
        "evidence must show the real tool dispatched:\n{joined}"
    );
}

/// V: the design's hallucination example (`read_files_fast`) is cleanly
/// rejected — unknown tool, no fuzzy match, no did-you-mean, nothing
/// executed.
#[test]
fn hallucination_is_cleanly_rejected() {
    let evidence = pass_verdict(task_69::run_scenario(&ctx_for(), "hallucination"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("clean unknown_tool rejection"),
        "evidence must show the clean rejection:\n{joined}"
    );
    assert!(
        joined.contains("nothing executed"),
        "evidence must show nothing executed:\n{joined}"
    );
}

// --- adversarial ---

/// A: the 10-name edit-distance-1 battery — including near misses of
/// the privileged `file_write` — is fully rejected, and a would-be
/// write through a near-miss name leaves no file: similarity is not
/// authority.
#[test]
fn near_miss_battery_all_rejected() {
    let evidence = pass_verdict(task_69::run_scenario(&ctx_for(), "near-miss-battery"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("10/10 near misses rejected"),
        "evidence must show the full battery rejected:\n{joined}"
    );
    assert!(
        joined.contains("created no file"),
        "evidence must show the near-miss write never executed:\n{joined}"
    );
}

/// A: a hallucinated name WITH valid args is rejected on the NAME
/// (unknown tool), while a real tool with bogus args fails on the ARGS
/// (unknown argument) — proving name lookup runs before arg parsing.
#[test]
fn name_lookup_runs_before_arg_parsing() {
    let evidence = pass_verdict(task_69::run_scenario(&ctx_for(), "name-before-args"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("rejected as unknown tool"),
        "evidence must show the name-first rejection:\n{joined}"
    );
    assert!(
        joined.contains("by_name[name] assert runs before validate_args"),
        "evidence must name the lookup-order mechanism:\n{joined}"
    );
}
