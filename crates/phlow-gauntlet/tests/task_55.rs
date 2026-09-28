//! Integration tests for task-55 (dissent escalation).
//!
//! The seam is ABSENT: diver has no multi-model answer comparison path
//! — adapters return answers, nothing compares two models' answers to
//! the same prompt, and no escalation record, dissent-rate counter, or
//! quarantine-threshold constant exists for models (`ai.security`'s
//! escalation/quarantine is file scanning, unrelated). Each test drives
//! the `task_55.lua` recon probe in headless Neovim against the REAL
//! diver Lua tree and asserts aspects of the honest `fail` verdict
//! (`where = "seam"`): 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_55;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required path from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_path(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-55: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-55: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-55: HOME is not set"))
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
        "gauntlet-diver-rtp-55-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&shim);
    std::fs::create_dir_all(&shim).unwrap_or_else(|e| panic!("task-55: cannot create shim: {e}"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, shim.join("lua"))
            .unwrap_or_else(|e| panic!("task-55: cannot symlink lua: {e}"));
        std::os::unix::fs::symlink(real.join("ai"), shim.join("ai"))
            .unwrap_or_else(|e| panic!("task-55: cannot symlink ai: {e}"));
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
        "gauntlet-task-55-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-55: cannot build Ctx: {e}"));
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
            "task-55 passed: comparison machinery was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the adapter probe completes and
/// reports the seam absence — `where = "seam"`, naming the missing
/// comparison path.
#[test]
fn probe_reports_seam_absence() {
    assert_eq!(task_55::ID, "task-55");
    assert_eq!(task_55::NAME, "dissent escalation");
    assert_eq!(task_55::KIND, TaskKind::NvimLua);
    assert_eq!(task_55::SCENARIOS.len(), 4, "2 validation + 2 adversarial");
    let (where_, how, evidence) = fail_verdict(task_55::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-55 must fail at the absent seam");
    let joined_ev = evidence.join("\n");
    assert!(
        joined_ev.contains("probed ai.harness.adapters.rose"),
        "an adapter module must actually load:\n{joined_ev}"
    );
    assert!(
        !joined_ev.contains("require failed"),
        "no module require may fail silently:\n{joined_ev}"
    );
    assert!(
        how.contains("no multi-model answer comparison path"),
        "the 'how' must name the absent comparison path: {how}"
    );
}

/// V: the verdict-and-security facet — `ai.harness.verdict.evaluate`
/// grades one run (no second model, no comparison), and `ai.security`'s
/// escalation/quarantine vocabulary is the control sample proving the
/// needle scan is not blind: it matches real file-scanning vocabulary,
/// unrelated to model-answer dissent.
#[test]
fn security_escalation_is_file_scanning_not_model_dissent() {
    let (where_, _how, evidence) =
        fail_verdict(task_55::run_scenario(&ctx_for(), "verdict-and-security"));
    assert_eq!(
        where_, "seam",
        "ai.security's file-scanning quarantine must not flip the verdict to 'recon'"
    );
    let joined = evidence.join("\n");
    assert!(
        joined.contains("ai.harness.verdict"),
        "evidence must show the verifier was probed:\n{joined}"
    );
    assert!(
        joined.contains("ai.security"),
        "evidence must show the control sample was probed:\n{joined}"
    );
    assert!(
        joined.contains("control sample"),
        "evidence must state the control-sample conclusion:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_55::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: the design's three required artifacts are documented absent — no
/// escalation record quoting both answers, no dissent-rate counter per
/// model, no quarantine-threshold named constant — and the fail-closed
/// facet records zero comparison-API hits explicitly.
#[test]
fn escalation_record_counter_and_threshold_all_absent() {
    let (_where_, _how, evidence) =
        fail_verdict(task_55::run_scenario(&ctx_for(), "no-escalation-record"));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("escalation record"),
        "evidence must document the missing escalation record:\n{joined}"
    );
    assert!(
        joined.contains("dissent-rate counter"),
        "evidence must document the missing dissent-rate counter:\n{joined}"
    );
    assert!(
        joined.contains("quarantine threshold"),
        "evidence must document the missing quarantine threshold:\n{joined}"
    );
    let (_where2, _how2, evidence2) =
        fail_verdict(task_55::run_scenario(&ctx_for(), "fail-closed-recon"));
    let joined2 = evidence2.join("\n");
    assert!(
        joined2.contains("zero comparison-API hits"),
        "fail-closed facet must record zero hits explicitly:\n{joined2}"
    );
}
