//! Integration tests for task-44 (egress filtering).
//!
//! The seam is ABSENT as designed: diver's `ai.harness` has no
//! URL-fetch tool — the registry holds no fetch/http/url tool, and
//! `register_builtins` registers adapters only — so the design's "tool
//! requests an internal address" scenarios have nothing to attach to.
//! The adjacent `ai.rose.http` is an operator-configured
//! model-endpoint client, not an agent URL-fetch tool: its URL check
//! is hostname-string based (no DNS resolution, no resolved-IP
//! filtering), and its curl transport refuses redirects
//! (`--max-redirs 0`) rather than re-checking per hop. Each test
//! drives the `task_44.lua` probe in headless Neovim against the REAL
//! diver Lua tree and asserts aspects of the honest `fail` verdict
//! (`where = "seam"`): 2 validation, 2 adversarial.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`, with
//! fallbacks to Matt's known tool paths. Missing binaries or directories
//! panic with a clear message: the gauntlet fails closed, never skips.
//!
//! Diver-owned finding: flagged in the probe evidence, never fixed on
//! gauntlet authority.

use phlow_gauntlet::tasks::task_44;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resolve a required directory from env or fallback. Panics (fail closed)
/// when the variable is empty or the path does not exist.
fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-44: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-44: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-44: HOME is not set"))
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
        "gauntlet-task-44-probe-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-44: cannot build Ctx: {e}"));
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
            "task-44 passed: resolved-destination egress filtering was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V: metadata contract pins the task; the probe completes and reports
/// the seam absence — `where = "seam"`, naming the missing URL-fetch
/// tool and the missing resolved-destination filtering.
#[test]
fn probe_reports_seam_absence() {
    assert_eq!(task_44::ID, "task-44");
    assert_eq!(task_44::NAME, "egress filtering");
    assert_eq!(task_44::KIND, TaskKind::NvimLua);
    let (where_, how, _evidence) = fail_verdict(task_44::run(&ctx_for()));
    assert_eq!(where_, "seam", "task-44 must fail at the absent seam");
    assert!(
        how.contains("no URL-fetch tool"),
        "the 'how' must name the missing URL-fetch tool: {how}"
    );
}

/// V: the probe inventoried the real registry and read the real
/// `ai/rose/http.lua` before concluding — the tool count is
/// reported, no fetch/http/url tool is found, and the http.lua
/// source was inspected for filtering evidence.
#[test]
fn probe_inventories_the_real_registry() {
    let (_where_, _how, evidence) = fail_verdict(task_44::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("harness registry tool count:"),
        "evidence must show the real registry inventory ran:\n{joined}"
    );
    assert!(
        joined.contains("no fetch/http/url tool in the harness registry"),
        "evidence must show the fetch-tool absence:\n{joined}"
    );
    assert!(
        joined.contains("read ai/rose/http.lua"),
        "evidence must show the real http.lua was read:\n{joined}"
    );
}

// --- adversarial ---

/// A: the verdict is a completed probe finding, not a probe crash — the
/// `where` is neither "bootstrap" (env/rtp failure) nor "lua-driver"
/// (unhandled Lua error). A crashing probe must never masquerade as the
/// seam finding.
#[test]
fn verdict_is_a_finding_not_a_probe_crash() {
    let (where_, _how, _evidence) = fail_verdict(task_44::run(&ctx_for()));
    assert!(
        where_ != "bootstrap" && where_ != "lua-driver",
        "the probe must run to completion; got where='{where_}'"
    );
}

/// A: the adjacent client's check is hostname-string only with no
/// resolved-IP filtering, and redirects are refused rather than
/// re-checked — DNS rebinding has no check to defeat because no
/// resolved-IP check exists at all.
#[test]
fn hostname_check_only_no_resolved_ip_filtering() {
    let (_where_, _how, evidence) = fail_verdict(task_44::run(&ctx_for()));
    let joined = evidence.join("\n");
    assert!(
        joined.contains("compares the hostname STRING"),
        "evidence must show the check is hostname-string based:\n{joined}"
    );
    assert!(
        joined.contains("--max-redirs 0"),
        "evidence must show redirects are refused, not re-checked:\n{joined}"
    );
    assert!(
        joined.contains("no resolved-IP check exists at all"),
        "evidence must show the resolved-IP check is absent:\n{joined}"
    );
}
