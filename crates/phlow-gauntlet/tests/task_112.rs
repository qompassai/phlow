//! Integration tests for task-112 (skill-document prompt injection).
//!
//! Four driver scenarios — 2 validation, 2 adversarial — run under
//! headless Neovim through diver's REAL `lua/ai/harness` include path
//! (trust-classified snapshots + seal), with a task-local fence-and-quote
//! prompt assembly (diver has no native optimizer-prompt render seam;
//! stated honestly in the driver header).
//!
//! The mock optimizer follows `OPTIMIZER:` directives wherever it sees
//! them unless fenced as data. Every scenario expects a pass: any
//! deviation is a breach naming the injection and the failed assembly
//! point. 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_112;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-112: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-112: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-112: HOME is not set"))
}

/// Process-local sequence so concurrent `ctx_for` calls never collide.
static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build a `Ctx` for one scenario with its own scratch directory.
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
        "gauntlet-task-112-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-112: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect a pass; panic on any failure or driver-harness error so a
/// breach can never masquerade as a finding.
fn expect_pass(scenario: &str) -> Vec<String> {
    let ctx = ctx_for(scenario);
    match task_112::run_scenario(&ctx, scenario) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-112 scenario '{scenario}' failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata pins the task; the clean scenario shows the baseline is
/// well-formed, the fence round-trips, and the negative control proves
/// the mock is injection-sensitive without the fence (non-vacuous).
#[test]
fn clean_baseline_and_negative_control() {
    assert_eq!(task_112::ID, "task-112");
    assert_eq!(task_112::NAME, "skill-document prompt injection");
    assert_eq!(task_112::KIND, TaskKind::NvimLua);
    let evidence = expect_pass("clean");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("diver probed:"),
        "evidence must record the diver SHA:\n{joined}"
    );
    assert!(
        joined.contains("negative control"),
        "evidence must show the non-vacuous sensitivity control:\n{joined}"
    );
    assert!(
        joined.contains("fence round-trip"),
        "evidence must show the fence round-trip:\n{joined}"
    );
}

/// V: all three poison fixtures leave the mock proposal byte-identical
/// to the clean baseline.
#[test]
fn poison_fixtures_inert() {
    let evidence = expect_pass("poison");
    let joined = evidence.join("\n");
    for fixture in [
        "directive-in-skill-doc",
        "fake-trajectory-in-skill-text",
        "directive-in-protected-section",
    ] {
        assert!(
            joined.contains(fixture),
            "evidence must name fixture {fixture}:\n{joined}"
        );
    }
    assert!(
        joined.contains("byte-identical"),
        "evidence must state byte-identical proposals:\n{joined}"
    );
}

// --- adversarial ---

/// A: instruction-like log content stays quoted (verbatim, inside the
/// fence) and does not move the proposal.
#[test]
fn log_content_stays_quoted() {
    let evidence = expect_pass("logquote");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("quoted"),
        "evidence must show the content stayed quoted:\n{joined}"
    );
}

/// A: an unknown OPTIMIZER: verb in the trusted channel is seen but
/// inert — behavior fields equal the baseline.
#[test]
fn unknown_directive_inert() {
    let evidence = expect_pass("unknown");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("inert"),
        "evidence must show the unknown verb stayed inert:\n{joined}"
    );
}

/// V: the driver performs zero deployed-surface writes. The work
/// directory is empty after the run (the driver writes no files), and
/// the driver source contains no file-writing calls outside
/// GAUNTLET_WORK_DIR.
#[test]
fn zero_deployed_surface_writes() {
    let ctx = ctx_for("clean");
    let work_dir = ctx.work_dir.clone();
    match task_112::run_scenario(&ctx, "clean") {
        TaskOutcome::Pass { .. } => {}
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-112 scenario 'clean' failed at {where_}: {how}\nevidence: {evidence:?}"),
    }
    // The driver wrote nothing: no regular files under the work dir.
    // (The harness itself may create empty scaffolding directories.)
    if work_dir.exists() {
        let mut files = Vec::new();
        let mut stack = vec![work_dir.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        assert!(
            files.is_empty(),
            "driver wrote files to work dir: {files:?}"
        );
    }
    // Static: the driver has no file-writing primitives.
    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lua/gauntlet/task_112.lua"),
    )
    .unwrap();
    for prim in ["io.open", "os.remove", "os.rename", "vim.fn.writefile"] {
        assert!(
            !src.contains(prim),
            "driver contains file-writing primitive: {prim}"
        );
    }
}
