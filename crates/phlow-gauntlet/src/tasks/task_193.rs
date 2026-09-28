// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 193 — scan-plan-apply sync (rust, V).
//!
//! The seam is the canonical → target skill sync: [`scan`] diffs the
//! canonical skill dir against the target dir, [`build_plan`] turns the
//! diff into an ordered add/update/remove plan, and [`apply`] executes
//! exactly that plan, backing up every replaced or removed file
//! byte-identical. A dry-run flag validates and prints the plan with
//! zero filesystem writes (asserted through the fs-access log).
//!
//! Adapted from Ghostex `packages/agent-sync`'s scan → plan → apply
//! pipeline (the Ghostex name misleads: it syncs skills/instructions,
//! not sessions). Ghostex links per-skill symlinks into agent CLIs;
//! this adaptation copies plain files into one target dir, because the
//! target is Matt's `~/workspace/skills` distribution. The backup
//! layout (mirror tree under the backup dir) is ours.

use crate::skill_sync::{
    ApplyOptions, FsAccess, PlanVerb, SyncPlan, apply, build_plan, scan, sha256_hex, sync,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::fs;
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-193";
/// Task name.
pub const NAME: &str = "scan-plan-apply sync";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["plan_apply_backup", "dry_run_writes_nothing"];

fn fixture(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: what.to_string(),
        detail: format!("task-193: {detail}"),
    }
}

/// Scratch dir unique to this case (tests in one process share a PID).
fn scratch_dir(case: &str) -> PathBuf {
    std::env::temp_dir().join(format!("gauntlet-193-{case}-{}", std::process::id()))
}

fn cleanup(dir: &Path) {
    // Best-effort: scratch cleanup must not fail the case.
    let _ = fs::remove_dir_all(dir);
}

fn write_file(dir: &Path, name: &str, content: &[u8]) -> Result<(), TaskDriverError> {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| fixture("mkdir", e.to_string()))?;
    }
    fs::write(&path, content).map_err(|e| fixture("write", e.to_string()))
}

fn read_file(dir: &Path, name: &str) -> Result<Vec<u8>, TaskDriverError> {
    fs::read(dir.join(name)).map_err(|e| fixture("read", e.to_string()))
}

/// Check the plan is exactly {update a.md, add b.md, remove c.md}.
fn check_plan_exact(plan: &SyncPlan, failures: &mut Vec<String>, evidence: &mut Vec<String>) {
    evidence.push(format!("plan: {:?}", plan.describe()));
    let want: Vec<(PlanVerb, &str)> = vec![
        (PlanVerb::Update, "a.md"),
        (PlanVerb::Add, "b.md"),
        (PlanVerb::Remove, "c.md"),
    ];
    let got: Vec<(PlanVerb, &str)> = plan
        .ops
        .iter()
        .map(|op| (op.verb.clone(), op.rel.as_str()))
        .collect();
    if got != want {
        failures.push(format!("plan {got:?}, want {want:?}"));
    }
}

/// Check the fs-access log shows exactly the plan: two writes (a, b),
/// one remove (c), and two backups (a and c — removed files are backed
/// up too).
fn check_fs_log(
    log: &[FsAccess],
    target: &Path,
    backup: &Path,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) {
    // Writes/removes live under the target; backups live under the
    // backup dir. Normalize each against its own root.
    let rel_of = |root: &Path, p: &PathBuf| {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    };
    let writes: Vec<String> = log
        .iter()
        .filter_map(|a| match a {
            FsAccess::Write { path } => Some(rel_of(target, path)),
            _ => None,
        })
        .collect();
    let removes: Vec<String> = log
        .iter()
        .filter_map(|a| match a {
            FsAccess::Remove { path } => Some(rel_of(target, path)),
            _ => None,
        })
        .collect();
    let backups: Vec<String> = log
        .iter()
        .filter_map(|a| match a {
            FsAccess::Backup { path } => Some(rel_of(backup, path)),
            _ => None,
        })
        .collect();
    if writes != ["a.md", "b.md"] {
        failures.push(format!("writes {writes:?}, want exactly [a.md, b.md]"));
    }
    if removes != ["c.md"] {
        failures.push(format!("removes {removes:?}, want exactly [c.md]"));
    }
    if backups != ["a.md", "c.md"] {
        failures.push(format!("backups {backups:?}, want exactly [a.md, c.md]"));
    }
    evidence.push(format!(
        "fs log: {} writes, {} removes, {} backups",
        writes.len(),
        removes.len(),
        backups.len()
    ));
}

/// Check backups are byte-identical to the pre-sync versions, and
/// that the new file b.md was not backed up.
fn check_backups(
    backup: &Path,
    a_old: &[u8],
    c_old: &[u8],
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    match read_file(backup, "a.md") {
        Ok(bytes) if bytes == a_old => {
            evidence.push(format!("backup a.md sha256={}", sha256_hex(&bytes)));
        }
        Ok(bytes) => failures.push(format!(
            "backup a.md differs from old version: {}",
            sha256_hex(&bytes)
        )),
        Err(e) => failures.push(format!("backup a.md missing: {e}")),
    }
    match read_file(backup, "c.md") {
        Ok(bytes) if bytes == c_old => {
            evidence.push("backup c.md byte-identical to removed version".to_string());
        }
        Ok(_) => failures.push("backup c.md differs from removed version".to_string()),
        Err(e) => failures.push(format!("backup c.md missing: {e}")),
    }
    if backup.join("b.md").exists() {
        failures.push("b.md was backed up but it is a new file".to_string());
    }
    Ok(())
}

/// Check the target converged: a.md == new, b.md added, c.md gone,
/// and no in-progress marker was left behind.
fn check_convergence(
    target: &Path,
    backup: &Path,
    a_new: &[u8],
    b_new: &[u8],
    failures: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    if read_file(target, "a.md")? != a_new {
        failures.push("target a.md != canonical new version".to_string());
    }
    if read_file(target, "b.md")? != b_new {
        failures.push("target b.md != canonical version".to_string());
    }
    if target.join("c.md").exists() {
        failures.push("target c.md still present after remove".to_string());
    }
    if backup.join(".sync-in-progress").exists() {
        failures.push("in-progress marker left behind".to_string());
    }
    Ok(())
}

/// Canonical {a(new), b}, target {a(old), c}: the plan must be
/// exactly {update a, add b, remove c}; apply executes exactly that
/// plan; the replaced `a` is backed up byte-identical to the old
/// version; the target ends at {a(new), b}.
fn case_plan_apply_backup() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir(CASES[0]);
    let _ = fs::remove_dir_all(&dir);
    let canonical = dir.join("canonical");
    let target = dir.join("target");
    let backup = dir.join("backup");
    fs::create_dir_all(&canonical).map_err(|e| fixture("mkdir", e.to_string()))?;
    fs::create_dir_all(&target).map_err(|e| fixture("mkdir", e.to_string()))?;
    let a_new = b"# skill a, version 2\n";
    let a_old = b"# skill a, version 1\n";
    let b_new = b"# skill b\n";
    let c_old = b"# skill c, target-only\n";
    write_file(&canonical, "a.md", a_new)?;
    write_file(&canonical, "b.md", b_new)?;
    write_file(&target, "a.md", a_old)?;
    write_file(&target, "c.md", c_old)?;

    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut log: Vec<FsAccess> = Vec::new();

    let plan = build_plan(
        &scan(&canonical).map_err(|e| fixture("scan-canonical", e.to_string()))?,
        &scan(&target).map_err(|e| fixture("scan-target", e.to_string()))?,
    );
    check_plan_exact(&plan, &mut failures, &mut evidence);
    let report = sync(
        &canonical,
        &target,
        &ApplyOptions::new(backup.clone()),
        &mut log,
    )
    .map_err(|e| fixture("sync", e.to_string()))?;
    if report.dry_run {
        failures.push("non-dry-run apply reported dry_run".to_string());
    }
    if report.applied.len() != 3 {
        failures.push(format!(
            "applied {} ops, want exactly the 3 planned",
            report.applied.len()
        ));
    }
    check_fs_log(&log, &target, &backup, &mut failures, &mut evidence);
    check_backups(&backup, a_old, c_old, &mut failures, &mut evidence)?;
    check_convergence(&target, &backup, a_new, b_new, &mut failures)?;

    cleanup(&dir);
    let mut report_out = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "plan_ops": plan.ops.len(),
            "writes": 2,
            "removes": 1,
            "backups": 2,
            "backend": "temp-fixture",
        }),
        evidence,
    );
    report_out.passed = failures.is_empty();
    report_out.failures = failures;
    Ok(report_out)
}

/// V2: dry-run → the plan is printed, but the fs-access log shows
/// zero mutations and the target dir is untouched (no writes, no
/// backup dir created).
fn case_dry_run_writes_nothing() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir(CASES[1]);
    let _ = fs::remove_dir_all(&dir);
    let canonical = dir.join("canonical");
    let target = dir.join("target");
    let backup = dir.join("backup");
    fs::create_dir_all(&canonical).map_err(|e| fixture("mkdir", e.to_string()))?;
    fs::create_dir_all(&target).map_err(|e| fixture("mkdir", e.to_string()))?;
    write_file(&canonical, "a.md", b"# skill a, version 2\n")?;
    write_file(&target, "a.md", b"# skill a, version 1\n")?;

    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut log: Vec<FsAccess> = Vec::new();

    let canonical_map = scan(&canonical).map_err(|e| fixture("scan", e.to_string()))?;
    let target_map = scan(&target).map_err(|e| fixture("scan", e.to_string()))?;
    let plan: SyncPlan = build_plan(&canonical_map, &target_map);
    let printed = plan.describe();
    let report = apply(
        &plan,
        &canonical,
        &canonical_map,
        &target,
        &ApplyOptions {
            dry_run: true,
            backup_dir: backup.clone(),
            kill_after_ops: None,
        },
        &mut log,
    )
    .map_err(|e| fixture("dry-run apply", e.to_string()))?;
    evidence.push(format!("dry-run plan printed: {printed:?}"));
    if !report.dry_run {
        failures.push("dry-run apply did not report dry_run".to_string());
    }
    if printed != ["update a.md"] {
        failures.push(format!(
            "dry-run printed {printed:?}, want [\"update a.md\"]"
        ));
    }
    if !log.is_empty() {
        failures.push(format!(
            "dry-run performed {} fs mutations: {log:?}",
            log.len()
        ));
    }
    if backup.exists() {
        failures.push("dry-run created the backup dir".to_string());
    }
    // The target is untouched: old content still in place.
    if read_file(&target, "a.md")? != b"# skill a, version 1\n" {
        failures.push("dry-run modified the target".to_string());
    }
    evidence.push(format!("fs-access log entries: {} (bar: 0)", log.len()));

    cleanup(&dir);
    let mut report_out = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "plan_printed": printed,
            "fs_mutations": log.len(),
            "backend": "temp-fixture",
        }),
        evidence,
    );
    report_out.passed = failures.is_empty();
    report_out.failures = failures;
    Ok(report_out)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "plan_apply_backup" => case_plan_apply_backup(),
        "dry_run_writes_nothing" => case_dry_run_writes_nothing(),
        _ => Err(fixture("case", format!("unknown case '{case}'"))),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// working scan → plan → apply pipeline with backups.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-193".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-193".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
