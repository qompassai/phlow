//! Integration tests for task-118 (policy launch enforcement, diver Fix 3).
//!
//! The defect: `supervisor.launch` (supervisor.lua line 182) goes straight
//! to `chosen.start` — `sup.policy` is stored but never consulted. The
//! fail-closed `policy.decide` (policy.lua lines 175-184) has no call sites.
//!
//! The tests assert the honest defect evidence: the deny-blocks, nil-policy,
//! and no-classification scenarios report `fail` with `where =
//! "fix-3-absent"`; the allow-launches scenario passes today (behaviorally
//! correct, for the wrong reason — `decide` was never consulted). 50/50
//! split: 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.

use phlow_gauntlet::tasks::task_118;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-118: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-118: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-118: HOME is not set"))
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
        "gauntlet-task-118-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-118: cannot build Ctx: {e}"));
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
    match task_118::run_scenario(&ctx, scenario) {
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

/// Expect the fix-3 gap record; panic on any other driver failure so
/// harness problems can never masquerade as findings.
fn expect_gap(scenario: &str) -> (String, Vec<String>) {
    match probe(scenario) {
        Probe::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(
                where_, "fix-3-absent",
                "task-118 scenario '{scenario}' failed in the driver harness, not the probe: {how}"
            );
            (how, evidence)
        }
        Probe::Pass { evidence } => panic!(
            "task-118 scenario '{scenario}' passed: policy was consulted, contradicting the probed defect\\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task, and a deny-all policy does NOT
/// block the launch today — the run proceeds to running.
#[test]
fn deny_all_policy_does_not_block_launch() {
    assert_eq!(task_118::ID, "task-118");
    assert_eq!(task_118::NAME, "policy-launch-enforcement");
    assert_eq!(task_118::KIND, TaskKind::NvimLua);
    let (how, evidence) = expect_gap("default");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("sup.policy") && joined.contains("never consulted"),
        "evidence must say sup.policy is stored but never consulted:\\n{joined}"
    );
    assert!(
        how.contains("launch") && how.contains("decide"),
        "the 'how' must demand decide at launch: {how}"
    );
}

/// V: allow-launches passes today — behaviorally correct, but for the
/// wrong reason: `decide` was never consulted. The test pins that the
/// pass is evidence-free of any policy decision.
#[test]
fn allow_policy_launch_passes_without_consultation() {
    let evidence = match probe("allow-launches") {
        Probe::Pass { evidence } => evidence,
        Probe::Fail { how, .. } => {
            panic!("task-118 'allow-launches' failed: even the allow path broke: {how}")
        }
    };
    let joined = evidence.join("\n");
    assert!(
        joined.contains("never consulted") || joined.contains("wrong reason"),
        "evidence must record that decide was never consulted:\\n{joined}"
    );
}

// --- adversarial ---

/// A: `decide(nil)` denies by itself (fail-closed), but a launch with
/// `sup.policy == nil` still proceeds — the function is safe, the call
/// site is missing.
#[test]
fn nil_policy_decides_deny_but_launch_proceeds() {
    let (how, evidence) = expect_gap("nil-policy");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("fail-closed") || joined.contains("no policy configured"),
        "evidence must show decide(nil) denies on its own:\\n{joined}"
    );
    assert!(
        joined.contains("no call site") || joined.contains("call site"),
        "evidence must name the missing call site:\\n{joined}"
    );
    assert!(
        how.contains("decide"),
        "the 'how' must demand decide at launch: {how}"
    );
}

/// A: launch never calls probe() and builds no policy request (source
/// evidence); the lying probe (remote=false while start does network I/O)
/// is banked as a trust-boundary finding, and the extensions-smuggling /
/// mutated-policy semantics are banked as acceptance criteria.
#[test]
fn no_classification_and_lying_probe_trust_boundary() {
    let (how, evidence) = expect_gap("no-classification");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("no policy reference"),
        "evidence must show launch() has no policy reference in source:\\n{joined}"
    );
    assert!(
        joined.contains("TRUST-BOUNDARY FINDING"),
        "evidence must bank the lying-probe trust boundary:\\n{joined}"
    );
    assert!(
        joined.contains("smuggl") || joined.contains("EXTENSIONS"),
        "evidence must bank the extensions-smuggling criterion:\\n{joined}"
    );
    assert!(
        how.contains("decide") && how.contains("launch"),
        "the 'how' must demand decide at launch time: {how}"
    );
}
