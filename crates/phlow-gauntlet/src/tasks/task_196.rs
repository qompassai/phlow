// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 196 — concurrent modification safety (rust, A).
//!
//! The seam is the apply phase under concurrent target edits. Two
//! guarantees: (1) a target file modified after the scan but before
//! apply is detected by re-hashing against the plan-time hash — apply
//! aborts with [`SyncError::ConcurrentModification`] before any
//! write, leaving the target untouched; (2) an apply killed midway
//! (simulated by the test-only `kill_after_ops` hook) leaves an
//! in-progress marker plus complete backups, so a rerun restores the
//! pre-sync state from backup, completes, and ends byte-equal to a
//! clean single apply.
//!
//! The scan-hash-vs-apply-hash check is ours (Ghostex's apply instead
//! recomputes the whole plan from a fresh scan right before running;
//! both close the same TOCTOU window, but re-hashing the planned ops
//! is cheaper than a full replan and keeps the refusal typed).

use crate::skill_sync::{
    ApplyOptions, FsAccess, SkillFile, SyncError, SyncPlan, apply, build_plan, scan, sync,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-196";
/// Task name.
pub const NAME: &str = "concurrent modification safety";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = ["concurrent_modification_aborts", "kill_midway_recovers"];

fn fixture(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: what.to_string(),
        detail: format!("task-196: {detail}"),
    }
}

/// Scratch dir unique to this case (tests in one process share a PID).
fn scratch_dir(case: &str) -> PathBuf {
    std::env::temp_dir().join(format!("gauntlet-196-{case}-{}", std::process::id()))
}

fn cleanup(dir: &Path) {
    // Best-effort: scratch cleanup must not fail the case.
    let _ = fs::remove_dir_all(dir);
}

fn write_file(dir: &Path, name: &str, content: &[u8]) -> Result<(), TaskDriverError> {
    fs::write(dir.join(name), content).map_err(|e| fixture("write", e.to_string()))
}

/// Canonical {a(new), b}, target {a(old)} — the shared fixture shape.
fn make_pair(dir: &Path) -> Result<(PathBuf, PathBuf), TaskDriverError> {
    let canonical = dir.join("canonical");
    let target = dir.join("target");
    fs::create_dir_all(&canonical).map_err(|e| fixture("mkdir", e.to_string()))?;
    fs::create_dir_all(&target).map_err(|e| fixture("mkdir", e.to_string()))?;
    write_file(&canonical, "a.md", b"# skill a, version 2\n")?;
    write_file(&canonical, "b.md", b"# skill b\n")?;
    write_file(&target, "a.md", b"# skill a, version 1\n")?;
    Ok((canonical, target))
}

/// After the abort: the intruder's bytes are intact (apply must not
/// "fix" a file it refused), b.md was never added, zero mutations
/// were logged, and no backup dir was created.
fn check_abort_clean(
    target: &Path,
    backup: &Path,
    intruder: &[u8],
    log: &[FsAccess],
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let after = fs::read(target.join("a.md")).map_err(|e| fixture("read", e.to_string()))?;
    if after != intruder {
        failures.push("apply changed the target despite aborting".to_string());
    }
    if target.join("b.md").exists() {
        failures.push("b.md was added despite the abort".to_string());
    }
    if !log.is_empty() {
        failures.push(format!(
            "{} fs mutations before the abort: {log:?}",
            log.len()
        ));
    }
    if backup.exists() {
        failures.push("backup dir created during an aborted apply".to_string());
    }
    evidence.push(format!(
        "fs-access log entries: {} (bar: 0); intruder bytes intact",
        log.len()
    ));
    Ok(())
}
/// A1: the target file is modified after the scan but before apply →
/// apply detects the hash mismatch, aborts with
/// `SyncError::ConcurrentModification`, and the target keeps the
/// externally-modified bytes (apply wrote nothing).
fn case_concurrent_modification_aborts() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir(CASES[0]);
    let _ = fs::remove_dir_all(&dir);
    let (canonical, target) = make_pair(&dir)?;
    let backup = dir.join("backup");

    let canonical_map = scan(&canonical).map_err(|e| fixture("scan", e.to_string()))?;
    let target_map = scan(&target).map_err(|e| fixture("scan", e.to_string()))?;
    let plan = build_plan(&canonical_map, &target_map);
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    if plan.ops.len() != 2 {
        failures.push(format!("plan has {} ops, want 2", plan.ops.len()));
    }
    evidence.push(format!("plan: {:?}", plan.describe()));

    // The concurrent edit lands after the scan, before apply.
    let intruder = b"# skill a, intruder edit\n";
    write_file(&target, "a.md", intruder)?;
    evidence.push("target a.md modified after scan (intruder edit)".to_string());

    let mut log: Vec<FsAccess> = Vec::new();
    match apply(
        &plan,
        &canonical,
        &canonical_map,
        &target,
        &ApplyOptions::new(backup.clone()),
        &mut log,
    ) {
        Err(SyncError::ConcurrentModification { rel }) if rel == "a.md" => {
            evidence.push(format!(
                "aborted with SyncError::ConcurrentModification for '{rel}'"
            ));
        }
        Err(other) => failures.push(format!("wrong outcome: {other}")),
        Ok(_) => failures.push("apply succeeded despite the concurrent edit".to_string()),
    }
    check_abort_clean(
        &target,
        &backup,
        intruder,
        &log,
        &mut failures,
        &mut evidence,
    )?;
    cleanup(&dir);
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "refusal": "ConcurrentModification",
            "fs_mutations": log.len(),
            "backend": "temp-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Kill phase: apply with the test-only hook fires `Killed` after the
/// first op. The mid-kill state must show one completed op, the marker
/// present, and the second op not run.
fn run_kill_phase(
    plan: &SyncPlan,
    canonical: &Path,
    canonical_map: &BTreeMap<String, SkillFile>,
    target: &Path,
    backup: &Path,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let mut log1: Vec<FsAccess> = Vec::new();
    let kill_opts = ApplyOptions {
        dry_run: false,
        backup_dir: backup.to_path_buf(),
        kill_after_ops: Some(1),
    };
    match apply(
        plan,
        canonical,
        canonical_map,
        target,
        &kill_opts,
        &mut log1,
    ) {
        Err(SyncError::Killed) => {
            evidence.push("first apply killed after 1 op (test hook)".to_string());
        }
        Err(other) => failures.push(format!("kill hook gave wrong outcome: {other}")),
        Ok(_) => failures.push("kill hook did not fire".to_string()),
    }
    if !backup.join(".sync-in-progress").exists() {
        failures.push("in-progress marker missing after the kill".to_string());
    }
    let mid_a = fs::read(target.join("a.md")).map_err(|e| fixture("read", e.to_string()))?;
    if mid_a != b"# skill a, version 2\n" {
        failures.push("first op did not complete before the kill".to_string());
    }
    if target.join("b.md").exists() {
        failures.push("second op ran despite the kill".to_string());
    }
    evidence.push("mid-kill state: a.md updated, b.md absent, marker present".to_string());
    Ok(())
}

/// Recovery phase: the rerun restores the pre-sync state from backup
/// (a restore entry in the fs log proves it), completes both ops, and
/// leaves no marker behind.
fn run_recovery_phase(
    plan: &SyncPlan,
    canonical: &Path,
    canonical_map: &BTreeMap<String, SkillFile>,
    target: &Path,
    backup: &Path,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(Vec<String>, usize, usize), TaskDriverError> {
    let mut log2: Vec<FsAccess> = Vec::new();
    let report = apply(
        plan,
        canonical,
        canonical_map,
        target,
        &ApplyOptions::new(backup.to_path_buf()),
        &mut log2,
    )
    .map_err(|e| fixture("rerun", e.to_string()))?;
    if report.restored != ["a.md"] {
        failures.push(format!(
            "rerun restored {:?}, want [\"a.md\"] from backup",
            report.restored
        ));
    }
    if report.applied.len() != 2 {
        failures.push(format!(
            "rerun applied {} ops, want 2",
            report.applied.len()
        ));
    }
    if backup.join(".sync-in-progress").exists() {
        failures.push("in-progress marker left behind after recovery".to_string());
    }
    let restores = log2
        .iter()
        .filter(|a| matches!(a, FsAccess::Restore { .. }))
        .count();
    evidence.push(format!(
        "rerun: {restores} restore entries in fs log, {} ops applied",
        report.applied.len()
    ));
    if restores == 0 {
        failures.push("rerun shows no restore from backup".to_string());
    }
    Ok((report.restored.clone(), report.applied.len(), restores))
}

/// Reference comparison: a clean single apply on an identical fresh
/// fixture must be byte-equal to the recovered tree, and the backup
/// must hold the pre-sync version of the replaced file.
fn check_reference_convergence(
    dir: &Path,
    target: &Path,
    backup: &Path,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let ref_dir = dir.join("reference");
    let (ref_canonical, ref_target) = make_pair(&ref_dir)?;
    let ref_backup = ref_dir.join("backup");
    let mut ref_log: Vec<FsAccess> = Vec::new();
    sync(
        &ref_canonical,
        &ref_target,
        &ApplyOptions::new(ref_backup),
        &mut ref_log,
    )
    .map_err(|e| fixture("reference sync", e.to_string()))?;
    for name in ["a.md", "b.md"] {
        let got = fs::read(target.join(name)).map_err(|e| fixture("read", e.to_string()))?;
        let want = fs::read(ref_target.join(name)).map_err(|e| fixture("read", e.to_string()))?;
        if got != want {
            failures.push(format!("final {name} differs from clean single apply"));
        }
    }
    let backed_a = fs::read(backup.join("a.md")).map_err(|e| fixture("read", e.to_string()))?;
    if backed_a != b"# skill a, version 1\n" {
        failures.push("backup a.md is not the pre-sync version".to_string());
    }
    evidence.push("final tree byte-equals a clean single apply".to_string());
    Ok(())
}

/// A2: apply is killed after the first op (test-only hook) → the
/// rerun finds the in-progress marker, restores the pre-sync state
/// from backup, completes, and the final tree is byte-equal to a
/// clean single apply on an identical fixture.
fn case_kill_midway_recovers() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir(CASES[1]);
    let _ = fs::remove_dir_all(&dir);
    let (canonical, target) = make_pair(&dir)?;
    let backup = dir.join("backup");

    let canonical_map = scan(&canonical).map_err(|e| fixture("scan", e.to_string()))?;
    let target_map = scan(&target).map_err(|e| fixture("scan", e.to_string()))?;
    let plan = build_plan(&canonical_map, &target_map);
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    run_kill_phase(
        &plan,
        &canonical,
        &canonical_map,
        &target,
        &backup,
        &mut failures,
        &mut evidence,
    )?;
    let (restored, applied, restores) = run_recovery_phase(
        &plan,
        &canonical,
        &canonical_map,
        &target,
        &backup,
        &mut failures,
        &mut evidence,
    )?;
    check_reference_convergence(&dir, &target, &backup, &mut failures, &mut evidence)?;

    cleanup(&dir);
    let mut report_out = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "restored": restored,
            "applied_on_rerun": applied,
            "restores_logged": restores,
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
        "concurrent_modification_aborts" => case_concurrent_modification_aborts(),
        "kill_midway_recovers" => case_kill_midway_recovers(),
        _ => Err(fixture("case", format!("unknown case '{case}'"))),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// concurrent-modification abort.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-196".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-196".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
