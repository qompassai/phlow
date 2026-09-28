//! Integration tests for task-60 (break-glass procedure).
//!
//! The seam is ABSENT: diver has no emergency-bypass (break-glass)
//! path — the approval state machine exits only via
//! `decide(approved|denied)` or `sweep_expired`, and `policy.lua`'s
//! contract is explicitly anti-bypass ("No adapter, provider, or MCP
//! server may bypass this module"). The `ai/security` "override" hits
//! are a control sample (prompt-injection/unicode-bidi scanner
//! vocabulary), classified unrelated. The design's question — "should
//! a break-glass procedure exist?" — is the documented finding,
//! banked for Matt. Each test drives the `task_60.lua` probe in
//! headless Neovim against the REAL diver Lua tree and asserts aspects
//! of the honest `fail` verdict (`where = "seam"`):
//! 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_60;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-60: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-60: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-60: HOME is not set"))
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
        "gauntlet-diver-rtp-60-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-60: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-60: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-60: cannot symlink ai: {e}"));
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
        "gauntlet-task-60-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-60: cannot build Ctx: {e}"));
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
            panic!("task-60 passed: a bypass path was invented, not found\nevidence: {evidence:?}")
        }
    }
}

// --- validation ---

/// V: metadata contract pins the task; the bypass-path scan completes
/// and reports the absent seam — `where = "seam"` — with the `how`
/// naming the approval state machine's closed exits and the
/// anti-bypass contract.
#[test]
fn probe_reports_absent_seam() {
    assert_eq!(task_60::ID, "task-60");
    assert_eq!(task_60::NAME, "break-glass procedure");
    assert_eq!(task_60::KIND, TaskKind::NvimLua);
    assert_eq!(task_60::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_verdict(task_60::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-60 must fail at the absent seam");
    let joined_ev = evidence.join("\n");
    assert!(
        joined_ev.contains("No adapter, provider, or MCP server may bypass this module"),
        "evidence must cite the anti-bypass contract:\n{joined_ev}"
    );
    assert!(
        how.contains("should a break-glass procedure exist"),
        "the 'how' must document the design question: {how}"
    );
}

/// V: the approval-exits facet — the state machine's only exits are
/// `decide(approved|denied)` and `sweep_expired`; no bypass entry point.
#[test]
fn approval_exits_are_closed() {
    let (where_, _how, evidence) =
        fail_verdict(task_60::run_scenario(&ctx_for(), "approval-exits-closed"));
    assert_eq!(
        where_, "seam",
        "closed exits must not flip the verdict to 'recon'"
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no bypass/override/emergency entry point"),
        "evidence must state the closed exits:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_60::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: the control sample — the `ai/security` "override" hits are
/// classified UNRELATED (prompt-injection/unicode-bidi scanner
/// vocabulary), never counted as break-glass machinery; the artifacts
/// facet documents all three required artifacts absent.
#[test]
fn control_sample_classified_not_counted() {
    let (_where_, _how, evidence) =
        fail_verdict(task_60::run_scenario(&ctx_for(), "bypass-path-scan"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("control sample"),
        "evidence must show the classified control sample:\n{joined}"
    );
    assert!(
        joined.contains("classified UNRELATED"),
        "the control sample must be classified, not counted:\n{joined}"
    );
    let (_where2, _how2, evidence2) = fail_verdict(task_60::run_scenario(
        &ctx_for(),
        "justification-artifacts-absent",
    ));
    let joined2 = evidence2.join("\n");
    assert!(
        joined2.contains("no such record type exists"),
        "evidence must document the absent justification record:\n{joined2}"
    );
    assert!(
        joined2.contains("no incident flag"),
        "evidence must document the absent incident flagging:\n{joined2}"
    );
}
