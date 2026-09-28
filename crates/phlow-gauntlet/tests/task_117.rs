//! Integration tests for task-117 (A2A completion integrity, diver Fix 2).
//!
//! The defect: the real callback contract in `ai/a2a/tasks.lua` (line 43)
//! is `on_done? fun(task: A2aTask)` — one argument — but the harness adapter
//! declares `function(result, task_err)` (adapters/a2a.lua line 63), so
//! `task_err` is always nil and the recorded outcome is always
//! `'completed'`. Failed remote tasks are recorded as completed.
//!
//! The tests drive the REAL adapters/a2a.lua with a stubbed transport and
//! assert the honest defect evidence. Today only the completed→completed
//! scenario passes (correct by accident: `task_err` is nil for every
//! invocation); everything else reports `fail` with `where = "fix-2-absent"`.
//! 50/50 split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_117;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-117: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-117: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-117: HOME is not set"))
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
        "gauntlet-task-117-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-117: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(120);
    ctx
}

/// Driver errors are harness failures, not findings: anything that is not
/// an honest acceptance verdict fails the test loudly.
enum Probe {
    Fail {
        where_: String,
        how: String,
        evidence: Vec<String>,
    },
    Pass {
        evidence: Vec<String>,
    },
}

fn probe(scenario: &str) -> Probe {
    let ctx = ctx_for(scenario);
    match task_117::run_scenario(&ctx, scenario) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => Probe::Fail {
            where_,
            how,
            evidence,
        },
        TaskOutcome::Pass { evidence } => Probe::Pass { evidence },
    }
}

/// Expect the fix-2 gap record; panic on any other driver failure so
/// harness problems can never masquerade as findings.
fn expect_gap(scenario: &str) -> (String, Vec<String>) {
    match probe(scenario) {
        Probe::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "fix-2-absent",
                "task-117 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        Probe::Pass { evidence } => panic!(
            "task-117 scenario '{scenario}' passed: the outcome was derived correctly, contradicting the probed defect\\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and a failed remote task is
/// recorded as completed — the one-argument callback always looks
/// successful to the adapter.
#[test]
fn failed_task_misreported_as_completed() {
    assert_eq!(task_117::ID, "task-117");
    assert_eq!(task_117::NAME, "a2a-completion-integrity");
    assert_eq!(task_117::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("adapters/a2a.lua"),
        "evidence must name adapters/a2a.lua lines 63-68:\\n{joined}"
    );
    assert!(
        joined.contains("failed") && joined.contains("completed"),
        "evidence must show failed -> completed:\\n{joined}"
    );
    assert!(
        how.contains("task_err") && how.contains("always"),
        "the 'how' must say task_err is always nil: {how}"
    );
}

/// V: completed→completed passes today — correctly by accident, since
/// `task_err` is nil for every invocation. The test pins the accident so a
/// future change that breaks the one honest mapping is caught.
#[test]
fn completed_maps_completed_by_accident() {
    let evidence = match probe("completed-maps-completed") {
        Probe::Pass { evidence } => evidence,
        Probe::Fail { how, .. } => {
            panic!(
                "task-117 'completed-maps-completed' failed: even the accidental mapping broke: {how}"
            )
        }
    };
    let joined = evidence.join("\n");
    assert!(
        joined.contains("by accident") || joined.contains("correct by accident"),
        "evidence must record that this mapping is accidental, not designed:\\n{joined}"
    );
}

// --- adversarial ---

/// A: rejected and canceled remote tasks are also recorded as completed
/// — the misreporting covers every non-completed terminal state.
#[test]
fn rejected_and_canceled_misreported_as_completed() {
    let (how, evidence) = expect_gap("rejected-canceled");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("rejected") && joined.contains("canceled"),
        "evidence must cover both states:\\n{joined}"
    );
    assert!(
        how.contains("task.state") || how.contains("task_err"),
        "the 'how' must name the broken derivation: {how}"
    );
}

/// A: garbage and nil states must fail closed (never completed), and a
/// double on_done must stay a clean no-op — exactly one run.finished.
/// Today the fail-closed half fails (both record completed) while the
/// idempotence half already holds.
#[test]
fn garbage_states_fail_closed_and_double_invoke_idempotent() {
    let (how, evidence) = expect_gap("garbage-nil-double");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("bogus-state"),
        "evidence must show the garbage state was recorded as completed:\\n{joined}"
    );
    assert!(
        joined.contains("fail-closed") || joined.contains("fail closed"),
        "evidence must state the fail-closed requirement:\\n{joined}"
    );
    assert!(
        joined.contains("idempotent finish holds"),
        "evidence must show double-invoke stayed a clean no-op:\\n{joined}"
    );
    assert!(
        how.contains("task.state") || how.contains("never completed"),
        "the 'how' must demand fail-closed semantics: {how}"
    );
}
