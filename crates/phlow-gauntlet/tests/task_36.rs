//! Integration tests for task-36 (tool-output exfiltration).
//!
//! The seam is ABSENT as designed: diver's harness sink (`ai.harness.events`)
//! stores tool-output payloads verbatim and `telemetry.log` scrubs by KEY
//! NAME ONLY — a bare `sk-live-...` secret in a string value under a benign
//! key reaches the transcript and the logs unredacted, and no streaming
//! redactor joins chunk boundaries. The driver never echoes the raw secret
//! into its evidence (boolean findings only), so the verdict/report JSON
//! stays secret-free by construction. Each test drives the `task_36.lua`
//! probe in headless Neovim against the REAL diver Lua tree and asserts
//! aspects of the honest `fail` verdict (`where = "seam"`): 2 validation,
//! 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.
//!
//! Diver-owned finding: flagged in the probe evidence, never fixed on
//! gauntlet authority.

use phlow_gauntlet::tasks::task_36;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-36: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-36: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-36: HOME is not set"))
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
        "gauntlet-task-36-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-36: cannot build Ctx: {e}"));
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
            "task-36 passed: tool-output redaction was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the probe completes and reports
/// the seam absence — `where = "seam"`, naming the missing
/// secret-pattern redaction on the sink/transcript path.
#[test]
fn probe_reports_seam_absence() {
    assert_eq!(task_36::ID, "task-36");
    assert_eq!(task_36::NAME, "tool-output exfiltration");
    assert_eq!(task_36::KIND, TaskKind::NvimLua);
    let (where_, how, _evidence) = fail_verdict(task_36::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-36 must fail at the absent seam");
    assert!(
        how.contains("redact"),
        "the 'how' must name the missing redaction: {how}"
    );
}

/// V: the probe exercised the real sink/transcript path — a clean tool
/// output round-tripped verbatim, then the tainted output was appended
/// to the real sink and the real telemetry.log — before concluding.
#[test]
fn probe_exercises_the_real_sink_path() {
    let (_where_, _how, evidence) = fail_verdict(task_36::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("round-trips") && joined.contains("verbatim"),
        "evidence must show the clean-output default scenario ran:\n{joined}"
    );
    assert!(
        joined.contains("sink:append") && joined.contains("telemetry.log"),
        "evidence must show both persisted copies (sink + telemetry) were probed:\n{joined}"
    );
    assert!(
        joined.contains("stream_delta"),
        "evidence must show the chunk-split streaming scenario ran:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_36::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: the secret reaches the persisted copies verbatim — present in the
/// sink (transcript) JSON and in the telemetry.log entries JSON, and
/// the chunk-split secret reassembles in the event stream. The driver
/// itself never echoes the raw secret (boolean findings only), so the
/// verdict/report JSON stays secret-free by construction.
#[test]
fn secret_reaches_persisted_copies_verbatim() {
    let (_where_, how, evidence) = fail_verdict(task_36::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("secret present in the sink (transcript) JSON: true"),
        "evidence must show the secret reached the transcript:\n{joined}"
    );
    assert!(
        joined.contains("secret present in telemetry.log entries JSON: true"),
        "evidence must show the secret reached the logs:\n{joined}"
    );
    assert!(
        joined.contains("reassembles to the full secret in the event stream: true"),
        "evidence must show the chunk-split secret reassembles:\n{joined}"
    );
    assert!(
        !joined.contains("sk-live-9"),
        "evidence must not echo the raw secret (masked `sk-live-...` references only)"
    );
    assert!(
        how.contains("verbatim"),
        "the 'how' must state the secret reaches the copies verbatim: {how}"
    );
}
