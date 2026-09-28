//! Integration tests for task-108 (cross-harness transfer).
//!
//! Four driver cases — 2 validation, 2 adversarial — exercised against
//! headless Neovim running diver's REAL skill load path
//! (`ai.mcp.skills`: setup → scan → get → tool_def). The Rust side
//! evolves F-bind skills on the clearly labeled scripted double,
//! freezes the exact bytes, and the lua driver round-trips them
//! through diver. Diver is read-only: the probed SHA is
//! c84352cc850d507df477706b9166b6541ebe9e1c.
//!
//! The context comes from `GAUNTLET_NVIM_BIN` / `GAUNTLET_DIVER_LUA`,
//! with fallbacks to Matt's known tool paths. Missing binaries or
//! directories panic with a clear message: the gauntlet fails closed,
//! never skips.

use phlow_gauntlet::tasks::task_108;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

fn required_dir(env_name: &str, fallback: &str) -> PathBuf {
    let raw = std::env::var(env_name).unwrap_or_else(|_| fallback.to_string());
    if raw.is_empty() {
        panic!("task-108: env {env_name} is set but empty");
    }
    let path = PathBuf::from(&raw);
    if !path.exists() {
        panic!(
            "task-108: required path does not exist: {} (from {env_name})",
            path.display()
        );
    }
    path
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| panic!("task-108: HOME is not set"))
}

static WORKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

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
        "gauntlet-task-108-{scenario}-{}-{seq}",
        std::process::id()
    ));
    let mut ctx = Ctx::new(nvim_bin, diver_lua, work_dir)
        .unwrap_or_else(|e| panic!("task-108: cannot build Ctx: {e}"));
    ctx.timeout = Duration::from_secs(180);
    ctx
}

/// Evolve once, assemble through diver once. Returns the frozen
/// variants and the out dir holding the assembled bytes.
fn assemble_once(ctx: &Ctx) -> (Vec<task_108::Variant>, PathBuf) {
    let work = ctx.work_dir.join(task_108::ID);
    let variants =
        task_108::prepare_variants(&work).unwrap_or_else(|e| panic!("task-108 prepare: {e}"));
    let variants_dir = work.join("variants");
    let out_dir = work.join("out");
    match task_108::run_lua_scenario(ctx, "assemble", &variants_dir, &out_dir) {
        TaskOutcome::Pass { .. } => (variants, out_dir),
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-108 assemble scenario failed at {where_}: {how}\nevidence: {evidence:?}"),
    }
}

fn assembled_bytes(out_dir: &Path, name: &str) -> Vec<u8> {
    task_108::read_assembled(out_dir, name).unwrap_or_else(|e| panic!("task-108 read: {e}"))
}

// --- validation ---

/// V1: every variant's procedure bytes after diver's real load path
/// are byte-identical to the frozen Rust bytes.
#[test]
fn assembly_byte_exact() {
    assert_eq!(task_108::ID, "task-108");
    assert_eq!(task_108::KIND, TaskKind::NvimLua);
    assert_eq!(
        task_108::DIVER_SHA,
        "c84352cc850d507df477706b9166b6541ebe9e1c"
    );
    let ctx = ctx_for("assemble");
    let (variants, out_dir) = assemble_once(&ctx);
    assert!(!variants.is_empty(), "must freeze at least one variant");
    for v in &variants {
        let back = assembled_bytes(&out_dir, &v.name);
        assert_eq!(
            back, v.body,
            "variant {}: diver-assembled bytes differ from frozen bytes",
            v.name
        );
    }
}

/// V2: both harnesses score at or above the no-skill baseline on the
/// same F-bind probe fixtures.
#[test]
fn both_harnesses_above_baseline() {
    let ctx = ctx_for("assemble");
    let (variants, out_dir) = assemble_once(&ctx);
    let baseline = task_108::baseline_score();
    for v in &variants {
        let rust_score =
            task_108::score_bytes(&v.body).unwrap_or_else(|e| panic!("task-108 score: {e}"));
        let lua_score = task_108::score_bytes(&assembled_bytes(&out_dir, &v.name))
            .unwrap_or_else(|e| panic!("task-108 score: {e}"));
        assert!(
            rust_score + 1e-12 >= baseline,
            "variant {}: rust harness {rust_score:.4} below baseline {baseline:.4}",
            v.name
        );
        assert!(
            lua_score + 1e-12 >= baseline,
            "variant {}: diver-assembled {lua_score:.4} below baseline {baseline:.4}",
            v.name
        );
    }
}

// --- adversarial ---

/// A1: the variant rank order by probe score is identical across
/// harnesses — assembly must not reorder, compress, or normalize.
#[test]
fn rank_order_preserved() {
    let ctx = ctx_for("assemble");
    let (variants, out_dir) = assemble_once(&ctx);
    let mut rust_rank: Vec<(String, u64)> = variants
        .iter()
        .map(|v| {
            let s = task_108::score_bytes(&v.body).unwrap();
            (v.name.clone(), (s * 1_000_000.0) as u64)
        })
        .collect();
    let mut lua_rank: Vec<(String, u64)> = variants
        .iter()
        .map(|v| {
            let s = task_108::score_bytes(&assembled_bytes(&out_dir, &v.name)).unwrap();
            (v.name.clone(), (s * 1_000_000.0) as u64)
        })
        .collect();
    rust_rank.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    lua_rank.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let rust_order: Vec<&str> = rust_rank.iter().map(|(n, _)| n.as_str()).collect();
    let lua_order: Vec<&str> = lua_rank.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        rust_order, lua_order,
        "variant rank order differs across harnesses"
    );
}

/// A2: a SKILL.md with an invalid skill name is rejected LOUDLY —
/// scan() reports the error and get() returns nil. Silent acceptance
/// would be the finding.
#[test]
fn invalid_name_rejected() {
    let ctx = ctx_for("invalid-name");
    let work = ctx.work_dir.join(task_108::ID);
    let variants_dir = task_108::prepare_invalid_variant(&work)
        .unwrap_or_else(|e| panic!("task-108 prepare-invalid: {e}"));
    let out_dir = work.join("out-bad");
    match task_108::run_lua_scenario(&ctx, "invalid-name", &variants_dir, &out_dir) {
        TaskOutcome::Pass { evidence } => {
            let joined = evidence.join("\n");
            assert!(
                joined.contains("rejected loudly"),
                "evidence must show the loud rejection:\n{joined}"
            );
        }
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!(
            "task-108 invalid-name scenario failed at {where_}: {how}\nevidence: {evidence:?}"
        ),
    }
}
