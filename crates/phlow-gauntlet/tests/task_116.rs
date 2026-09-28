//! Integration tests for task-116 (goal propagation, diver Fix 1).
//!
//! The defect: `types.validate_run_spec` (types.lua line 229) REQUIRES
//! `spec.goal`, but `supervisor.create` (supervisor.lua line 116) never copies
//! it into the run table — while `adapters/a2a.lua` (line 61) submits with
//! `message = run.goal`, i.e. nil. Every A2A run currently sends an empty
//! message.
//!
//! The tests assert the honest defect evidence: every scenario reports
//! `fail` with `where = "fix-1-absent"` and names the gap, except where the
//! current behavior happens to hold (none today — the field is dropped, not
//! filtered, so no scenario passes).
//! 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_116;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-116: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-116: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-116: HOME is not set"))
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
        "gauntlet-task-116-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-116: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Expect the fix-1 gap record; panic on any other outcome (a pass or a
/// driver-harness failure) so nothing masquerades as a finding.
fn expect_gap(scenario: &str) -> (String, Vec<String>) {
    let ctx = ctx_for(scenario);
    match task_116::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "fix-1-absent",
                "task-116 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-116 scenario '{scenario}' passed: the goal survived, contradicting the probed defect\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and the default scenario shows the
/// created run table is missing the validated goal (supervisor.lua M.create).
#[test]
fn default_goal_dropped_by_create() {
    assert_eq!(task_116::ID, "task-116");
    assert_eq!(task_116::NAME, "goal-propagation");
    assert_eq!(task_116::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("supervisor.lua"),
        "evidence must name supervisor.lua:\\n{joined}"
    );
    assert!(
        joined.contains("run.goal"),
        "evidence must show run.goal is nil after create:\\n{joined}"
    );
    assert!(
        how.contains("spec.goal") && how.contains("create"),
        "the 'how' must name the dropped field: {how}"
    );
}

/// V: the A2A adapter receives nil at start — the submitted message is
/// gone before it ever leaves the process (adapters/a2a.lua line 61).
#[test]
fn launch_delivers_nil_to_adapter() {
    let (how, evidence) = expect_gap("launch-delivers-goal");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("adapters/a2a.lua"),
        "evidence must name adapters/a2a.lua line 61:\\n{joined}"
    );
    assert!(
        how.contains("run.goal") || how.contains("nil"),
        "the 'how' must say the adapter received nil: {how}"
    );
}

// --- adversarial ---

/// A: hostile prompt-injection text is dropped, not filtered — no
/// filtering layer exists at this seam to mutate it. The defect is loss,
/// not sanitization, and the driver must not claim otherwise.
#[test]
fn injection_text_dropped_not_filtered() {
    let (how, evidence) = expect_gap("injection-pass-through");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no filtering layer"),
        "evidence must state there is no filtering layer here:\\n{joined}"
    );
    assert!(
        how.contains("create") && how.contains("spec.goal"),
        "the 'how' must still name the create-time drop: {how}"
    );
}

/// A: unicode and edge whitespace do not survive either — the fidelity
/// gap is total loss of the field, so byte-identical transport is
/// unverifiable until the field exists.
#[test]
fn unicode_whitespace_lost_with_field() {
    let (_how, evidence) = expect_gap("unicode-whitespace");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("unicode") || joined.contains("caf"),
        "evidence must show the unicode probe did not survive:\\n{joined}"
    );
}
