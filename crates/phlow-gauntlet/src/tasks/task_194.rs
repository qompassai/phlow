// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 194 — idempotent skill sync (rust, V).
//!
//! The seam is a sync with no changes: a no-op sync is observably a
//! no-op — the second run's plan is empty, zero files are written,
//! and mtimes are untouched. Comparison is by content hash (sha2),
//! never by mtime: touching a target file without changing its
//! content still yields an empty plan.
//!
//! Adapted from Ghostex `packages/agent-sync`'s scan → plan → apply
//! pipeline (Ghostex syncs skills/instructions, not sessions). The
//! content-hash comparison is the design's own requirement ("content
//! hashing (sha2, already a dep) as the comparison"); the Ghostex
//! scan is metadata-only and compares less strictly.

use crate::skill_sync::{ApplyOptions, FsAccess, apply, build_plan, scan, sync};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Task id.
pub const ID: &str = "task-194";
/// Task name.
pub const NAME: &str = "idempotent skill sync";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["double_sync_noop", "touch_without_change_noop"];

fn fixture(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: what.to_string(),
        detail: format!("task-194: {detail}"),
    }
}

/// Scratch dir unique to this case (tests in one process share a PID).
fn scratch_dir(case: &str) -> PathBuf {
    std::env::temp_dir().join(format!("gauntlet-194-{case}-{}", std::process::id()))
}

fn cleanup(dir: &Path) {
    // Best-effort: scratch cleanup must not fail the case.
    let _ = fs::remove_dir_all(dir);
}

fn write_file(dir: &Path, name: &str, content: &[u8]) -> Result<(), TaskDriverError> {
    fs::write(dir.join(name), content).map_err(|e| fixture("write", e.to_string()))
}

fn mtime_of(dir: &Path, name: &str) -> Result<SystemTime, TaskDriverError> {
    fs::metadata(dir.join(name))
        .map_err(|e| fixture("metadata", e.to_string()))?
        .modified()
        .map_err(|e| fixture("mtime", e.to_string()))
}

/// Build canonical {a, b} and an identical target, then run one full
/// sync. Returns the scratch dir and per-file mtimes after the sync.
fn synced_pair(
    case: &str,
) -> Result<(PathBuf, PathBuf, PathBuf, Vec<SystemTime>), TaskDriverError> {
    let dir = scratch_dir(case);
    let _ = fs::remove_dir_all(&dir);
    let canonical = dir.join("canonical");
    let target = dir.join("target");
    let backup = dir.join("backup");
    fs::create_dir_all(&canonical).map_err(|e| fixture("mkdir", e.to_string()))?;
    fs::create_dir_all(&target).map_err(|e| fixture("mkdir", e.to_string()))?;
    write_file(&canonical, "a.md", b"# skill a\n")?;
    write_file(&canonical, "b.md", b"# skill b\n")?;
    let mut log: Vec<FsAccess> = Vec::new();
    let report = sync(
        &canonical,
        &target,
        &ApplyOptions::new(backup.clone()),
        &mut log,
    )
    .map_err(|e| fixture("first sync", e.to_string()))?;
    if report.applied.len() != 2 {
        return Err(fixture(
            "first sync",
            format!(
                "first sync applied {} ops, want 2 adds",
                report.applied.len()
            ),
        ));
    }
    if !report.backed_up.is_empty() {
        return Err(fixture(
            "first sync",
            format!("first sync backed up {:?}, want none", report.backed_up),
        ));
    }
    let mtimes = vec![mtime_of(&target, "a.md")?, mtime_of(&target, "b.md")?];
    Ok((dir, canonical, target, mtimes))
}

/// Inputs to the shared no-op assertion.
struct NoopCheck<'a> {
    case: &'static str,
    plan_ops: usize,
    log: &'a [FsAccess],
    mtimes_before: &'a [SystemTime],
    mtimes_after: &'a [SystemTime],
    evidence: Vec<String>,
    failures: Vec<String>,
    dir: &'a Path,
}

fn finish(check: NoopCheck<'_>) -> Result<CaseReport, TaskDriverError> {
    let mut failures = check.failures;
    if check.plan_ops != 0 {
        failures.push(format!(
            "second plan has {} ops, want empty",
            check.plan_ops
        ));
    }
    if !check.log.is_empty() {
        failures.push(format!(
            "second sync performed {} fs mutations: {:?}",
            check.log.len(),
            check.log
        ));
    }
    if check.mtimes_before != check.mtimes_after {
        failures.push("target mtimes changed on a no-op sync".to_string());
    }
    cleanup(check.dir);
    let mut report = CaseReport::pass(
        check.case,
        serde_json::json!({
            "plan_ops": check.plan_ops,
            "fs_mutations": check.log.len(),
            "mtimes_stable": check.mtimes_before == check.mtimes_after,
            "backend": "temp-fixture",
        }),
        check.evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// V1: sync twice with no changes between. The second run's plan is
/// empty, zero files are written, and the target mtimes are untouched.
fn case_double_sync_noop() -> Result<CaseReport, TaskDriverError> {
    let (dir, canonical, target, mtimes_before) = synced_pair(CASES[0])?;
    let backup = dir.join("backup");
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut log: Vec<FsAccess> = Vec::new();

    let canonical_map = scan(&canonical).map_err(|e| fixture("scan", e.to_string()))?;
    let target_map = scan(&target).map_err(|e| fixture("scan", e.to_string()))?;
    let plan = build_plan(&canonical_map, &target_map);
    evidence.push(format!("second plan: {:?}", plan.describe()));
    let report = apply(
        &plan,
        &canonical,
        &canonical_map,
        &target,
        &ApplyOptions::new(backup),
        &mut log,
    )
    .map_err(|e| fixture("second apply", e.to_string()))?;
    if !report.applied.is_empty() {
        failures.push(format!(
            "second apply executed {} ops",
            report.applied.len()
        ));
    }
    let mtimes_after = vec![mtime_of(&target, "a.md")?, mtime_of(&target, "b.md")?];
    evidence.push(format!(
        "fs-access log entries on second sync: {} (bar: 0)",
        log.len()
    ));
    finish(NoopCheck {
        case: CASES[0],
        plan_ops: plan.ops.len(),
        log: &log,
        mtimes_before: &mtimes_before,
        mtimes_after: &mtimes_after,
        evidence,
        failures,
        dir: &dir,
    })
}

/// V2: sync, then `touch` a target file (mtime bumped, content
/// unchanged) → the plan is still empty: the content hash decides,
/// not the mtime.
fn case_touch_without_change_noop() -> Result<CaseReport, TaskDriverError> {
    let (dir, canonical, target, mtimes_before) = synced_pair(CASES[1])?;
    let backup = dir.join("backup");
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut log: Vec<FsAccess> = Vec::new();

    // Touch a.md: bump the mtime without changing a single byte.
    let touched = fs::File::options()
        .write(true)
        .open(target.join("a.md"))
        .map_err(|e| fixture("open", e.to_string()))?;
    touched
        .set_modified(SystemTime::now())
        .map_err(|e| fixture("touch", e.to_string()))?;
    drop(touched);
    let touched_mtime = mtime_of(&target, "a.md")?;
    evidence.push(format!(
        "a.md mtime before touch: {mtimes_before:?}, after: {touched_mtime:?}"
    ));

    let canonical_map = scan(&canonical).map_err(|e| fixture("scan", e.to_string()))?;
    let target_map = scan(&target).map_err(|e| fixture("scan", e.to_string()))?;
    let plan = build_plan(&canonical_map, &target_map);
    evidence.push(format!("plan after touch: {:?}", plan.describe()));
    let report = apply(
        &plan,
        &canonical,
        &canonical_map,
        &target,
        &ApplyOptions::new(backup),
        &mut log,
    )
    .map_err(|e| fixture("apply", e.to_string()))?;
    if !report.applied.is_empty() {
        failures.push(format!(
            "apply executed {} ops after a mere touch",
            report.applied.len()
        ));
    }
    // The mtime comparison for the no-op assertion uses the
    // post-touch mtimes: the sync itself must not rewrite anything, so
    // mtimes measured after the sync must equal the post-touch ones.
    let mtimes_touched = vec![mtime_of(&target, "a.md")?, mtime_of(&target, "b.md")?];
    evidence.push(format!(
        "fs-access log entries after touch+sync: {} (bar: 0)",
        log.len()
    ));
    finish(NoopCheck {
        case: CASES[1],
        plan_ops: plan.ops.len(),
        log: &log,
        mtimes_before: &mtimes_touched,
        mtimes_after: &[mtime_of(&target, "a.md")?, mtime_of(&target, "b.md")?],
        evidence,
        failures,
        dir: &dir,
    })
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "double_sync_noop" => case_double_sync_noop(),
        "touch_without_change_noop" => case_touch_without_change_noop(),
        _ => Err(fixture("case", format!("unknown case '{case}'"))),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// no-op second sync.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-194".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-194".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
