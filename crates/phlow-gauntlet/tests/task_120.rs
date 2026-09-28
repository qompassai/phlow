//! Integration tests for task-120 (`M.run` failure-path legality, diver Fix 6).
//!
//! The defect: `init.lua M.run` (line 83) calls `supervisor.finish(...,
//! 'failed')` for ANY failure — including failures that happened BEFORE the
//! created→queued transition inside `start_run`. `finish` then attempts the
//! illegal created→failed transition (`types.TRANSITIONS.created` permits
//! only queued/cancelled), emitting a spurious `diagnostic.invalid_transition`
//! and losing the intended failure outcome.
//!
//! The tests assert the honest defect evidence: the pre-queued-failure and
//! not-set-up scenarios report `fail` with `where = "fix-6-absent"`; the
//! unknown-adapter (default) and queued-failure scenarios pass today
//! (queued→failed is legal, reason preserved). 50/50 split: 2 validation,
//! 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_120;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-120: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-120: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-120: HOME is not set"))
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
        "gauntlet-task-120-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-120: cannot build Ctx: {e}"));
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
    match task_120::run_scenario(&ctx, scenario) {
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

// --- validation ---

/// V: metadata contract pins the task, and the unknown-adapter path is
/// legal today: start_run fails AFTER the run is queued, so queued→failed
/// preserves the reason with no spurious diagnostic.
#[test]
fn unknown_adapter_after_queued_is_legal() {
    assert_eq!(task_120::ID, "task-120");
    assert_eq!(task_120::NAME, "run-failure-legality");
    assert_eq!(task_120::KIND, TaskKind::NvimLua);
    let evidence = match probe("default") {
        Probe::Pass { evidence } => evidence,
        Probe::Fail { how, .. } => {
            panic!("task-120 'default' failed: the legal queued->failed path broke: {how}")
        }
    };
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no spurious diagnostic") || joined.contains("spurious"),
        "evidence must confirm no spurious diagnostic:\\n{joined}"
    );
}

/// V: an adapter start that returns an error after the run is queued lands
/// the run in failed with the reason preserved on run.finished.
#[test]
fn queued_start_failure_preserves_reason() {
    let evidence = match probe("queued-failure") {
        Probe::Pass { evidence } => evidence,
        Probe::Fail { how, .. } => {
            panic!("task-120 'queued-failure' failed: reason preservation broke: {how}")
        }
    };
    let joined = evidence.join("\n");
    assert!(
        joined.contains("run.finished") || joined.contains("reason"),
        "evidence must show the preserved reason:\\n{joined}"
    );
}

// --- adversarial ---

/// A: a failure that predates the created→queued transition must not emit
/// a diagnostic and must leave the run in created. Today the driver shows
/// the illegal transition is attempted: spurious diagnostic emitted, the
/// intended outcome lost.
#[test]
fn pre_queued_failure_is_illegal_today() {
    let (how, evidence) = match probe("pre-queued-failure") {
        Probe::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "fix-6-absent",
                "task-120 'pre-queued-failure' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        Probe::Pass { evidence } => panic!(
            "task-120 'pre-queued-failure' passed: the pre-queued path is legal, contradicting the probed defect\\nevidence: {evidence:?}"
        ),
    };
    let joined = evidence.join("\n");
    assert!(
        joined.contains("illegal") || joined.contains("created->failed"),
        "evidence must name the illegal created->failed transition:\\n{joined}"
    );
    assert!(
        how.contains("diagnostic") || how.contains("no diagnostic"),
        "the 'how' must state the no-diagnostic acceptance criterion: {how}"
    );
}

/// A: `M.run` before setup returns the clear "not set up" error (passes
/// today), but a raising adapter start propagates out of `harness.run`
/// uncaught — launch calls `chosen.start` with no pcall, so the run is
/// left in queued instead of landing in failed with the reason preserved.
#[test]
fn not_set_up_clear_error_but_raising_start_uncontained() {
    let (how, evidence) = match probe("not-set-up") {
        Probe::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "fix-6-absent",
                "task-120 'not-set-up' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        Probe::Pass { evidence } => panic!(
            "task-120 'not-set-up' passed: the raising start stayed contained, contradicting the probed defect\\nevidence: {evidence:?}"
        ),
    };
    let joined = evidence.join("\n");
    assert!(
        joined.contains("not set up") || joined.contains("M.run before setup"),
        "evidence must show the clear not-set-up error:\\n{joined}"
    );
    assert!(
        joined.contains("uncaught") || joined.contains("propagated"),
        "evidence must show the raise propagated out of harness.run:\\n{joined}"
    );
    assert!(
        how.contains("pcall") || how.contains("contain"),
        "the 'how' must demand containment: {how}"
    );
}
