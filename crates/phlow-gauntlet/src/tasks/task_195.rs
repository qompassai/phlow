// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 195 — skill supply-chain refusal (rust, A).
//!
//! The seam is the canonical → target apply with a hostile canonical
//! dir: upstream skill content is untrusted. Path traversal in a skill
//! name is refused with [`SyncError::Traversal`] before any write;
//! symlinks are never followed (declared contract: refused with
//! [`SyncError::SymlinkRefused`], not copied, not dereferenced);
//! executable bits are stripped on write (every synced file lands at
//! mode `0o644`).
//!
//! The symlink contract is ours and is deliberately the strict one:
//! Ghostex's agent-sync *creates* symlinks into agent CLIs, but there
//! the links point at the trusted canonical dir. Here the canonical
//! dir itself is the untrusted input, so a link is an escape attempt,
//! not a feature. Refusing (rather than copying the link or its
//! target) keeps the refusal visible to the operator instead of
//! silently materializing attacker-chosen content.

use crate::skill_sync::{
    ApplyOptions, FsAccess, PlanOp, PlanVerb, SkillFile, SyncError, apply, sha256,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-195";
/// Task name.
pub const NAME: &str = "skill supply-chain refusal";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 3 adversarial.
pub const CASES: [&str; 3] = [
    "traversal_refused",
    "symlink_not_followed",
    "exec_bit_stripped",
];

fn fixture(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: what.to_string(),
        detail: format!("task-195: {detail}"),
    }
}

/// Scratch dir unique to this case (tests in one process share a PID).
fn scratch_dir(case: &str) -> PathBuf {
    std::env::temp_dir().join(format!("gauntlet-195-{case}-{}", std::process::id()))
}

fn cleanup(dir: &Path) {
    // Best-effort: scratch cleanup must not fail the case.
    let _ = fs::remove_dir_all(dir);
}

fn finish(
    case: &'static str,
    evidence: Vec<String>,
    failures: Vec<String>,
    dir: &Path,
) -> Result<CaseReport, TaskDriverError> {
    cleanup(dir);
    let mut report = CaseReport::pass(
        case,
        serde_json::json!({
            "backend": "temp-fixture",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// After a refused apply: nothing escaped the target root and no
/// backup dir was created.
fn check_no_escape(
    dir: &Path,
    outside: &Path,
    backup: &Path,
    log: &[FsAccess],
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    if !log.is_empty() {
        failures.push(format!(
            "{} fs mutations before refusal: {log:?}",
            log.len()
        ));
    }
    // The escape target would have landed at dir/evil.sh (two levels
    // up from target/).
    if dir.join("evil.sh").exists() {
        failures.push("ESCAPE: dir/evil.sh was created".to_string());
    }
    for entry in fs::read_dir(outside).map_err(|e| fixture("readdir", e.to_string()))? {
        let entry = entry.map_err(|e| fixture("readdir", e.to_string()))?;
        failures.push(format!(
            "ESCAPE: file outside target root: {}",
            entry.path().display()
        ));
    }
    if backup.exists() {
        failures.push("backup dir created during a refused apply".to_string());
    }
    evidence.push(format!(
        "fs-access log entries: {} (bar: 0); outside-check dir clean",
        log.len()
    ));
    Ok(())
}

/// A1: a canonical entry named `../../evil.sh` → apply refuses with
/// `SyncError::Traversal`, zero writes, and nothing lands outside the
/// target root. (A real `readdir` can never produce a `..` name, so
/// the guard is exercised at the join layer, where every write
/// funnels through.)
fn case_traversal_refused() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir(CASES[0]);
    let _ = fs::remove_dir_all(&dir);
    let canonical = dir.join("canonical");
    let target = dir.join("target");
    let backup = dir.join("backup");
    let outside = dir.join("outside-check");
    for d in [&canonical, &target, &outside] {
        fs::create_dir_all(d).map_err(|e| fixture("mkdir", e.to_string()))?;
    }
    let payload = b"#!/bin/sh\necho pwned\n";

    // Hostile entry injected at the plan layer: the rel escapes the root.
    let mut hostile_canonical: BTreeMap<String, SkillFile> = BTreeMap::new();
    hostile_canonical.insert(
        "../../evil.sh".to_string(),
        SkillFile {
            hash: sha256(payload),
            len: payload.len() as u64,
            is_symlink: false,
        },
    );
    let plan = crate::skill_sync::SyncPlan {
        ops: vec![PlanOp {
            verb: PlanVerb::Add,
            rel: "../../evil.sh".to_string(),
            expected_hash: None,
        }],
    };

    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut log: Vec<FsAccess> = Vec::new();
    match apply(
        &plan,
        &canonical,
        &hostile_canonical,
        &target,
        &ApplyOptions::new(backup.clone()),
        &mut log,
    ) {
        Err(SyncError::Traversal { rel }) if rel == "../../evil.sh" => {
            evidence.push(format!("refused with SyncError::Traversal for '{rel}'"));
        }
        Err(other) => failures.push(format!("wrong refusal: {other}")),
        Ok(_) => failures.push("hostile plan applied without refusal".to_string()),
    }
    check_no_escape(&dir, &outside, &backup, &log, &mut failures, &mut evidence)?;
    finish(CASES[0], evidence, failures, &dir)
}

/// After a symlink refusal: no link.md in the target, the secret
/// bytes never entered the target tree, and the benign good.md was
/// not written either (the refusal aborts the whole apply).
fn check_no_leak(
    target: &Path,
    secret_bytes: &[u8],
    log: &[FsAccess],
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    if target.join("link.md").exists() {
        failures.push("target/link.md exists: the symlink was materialized".to_string());
    }
    for entry in fs::read_dir(target).map_err(|e| fixture("readdir", e.to_string()))? {
        let entry = entry.map_err(|e| fixture("readdir", e.to_string()))?;
        let bytes = fs::read(entry.path()).map_err(|e| fixture("read", e.to_string()))?;
        if bytes == secret_bytes {
            failures.push(format!(
                "secret bytes leaked into {}",
                entry.path().display()
            ));
        }
    }
    if target.join("good.md").exists() {
        failures.push("good.md was written despite the symlink refusal".to_string());
    }
    if !log.is_empty() {
        failures.push(format!("writes happened before refusal: {log:?}"));
    }
    evidence.push(format!("fs-access log entries: {} (bar: 0)", log.len()));
    Ok(())
}

/// A2: a canonical symlink pointing outside the target root is not
/// followed — apply refuses with `SyncError::SymlinkRefused` (our
/// declared contract: symlinks are never followed, never copied).
fn case_symlink_not_followed() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir(CASES[1]);
    let _ = fs::remove_dir_all(&dir);
    let canonical = dir.join("canonical");
    let target = dir.join("target");
    let backup = dir.join("backup");
    let secret = dir.join("secret.txt");
    for d in [&canonical, &target] {
        fs::create_dir_all(d).map_err(|e| fixture("mkdir", e.to_string()))?;
    }
    fs::write(&secret, b"attacker-controlled bytes\n")
        .map_err(|e| fixture("write", e.to_string()))?;
    fs::write(canonical.join("good.md"), b"# good\n")
        .map_err(|e| fixture("write", e.to_string()))?;
    // The hostile link: points at a file outside both roots.
    symlink(&secret, canonical.join("link.md")).map_err(|e| fixture("symlink", e.to_string()))?;

    let canonical_map =
        crate::skill_sync::scan(&canonical).map_err(|e| fixture("scan", e.to_string()))?;
    let link_entry = canonical_map.get("link.md");
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    match link_entry {
        Some(e) if e.is_symlink => {
            evidence.push("scan indexed link.md as a symlink (not followed)".to_string());
        }
        other => failures.push(format!("scan did not flag the symlink: {other:?}")),
    }
    let target_map =
        crate::skill_sync::scan(&target).map_err(|e| fixture("scan", e.to_string()))?;
    let plan = crate::skill_sync::build_plan(&canonical_map, &target_map);
    evidence.push(format!("plan: {:?}", plan.describe()));

    let mut log: Vec<FsAccess> = Vec::new();
    match apply(
        &plan,
        &canonical,
        &canonical_map,
        &target,
        &ApplyOptions::new(backup),
        &mut log,
    ) {
        Err(SyncError::SymlinkRefused { rel }) if rel == "link.md" => {
            evidence.push(format!(
                "refused with SyncError::SymlinkRefused for '{rel}'"
            ));
        }
        Err(other) => failures.push(format!("wrong refusal: {other}")),
        Ok(_) => failures.push("symlink plan applied without refusal".to_string()),
    }
    check_no_leak(
        &target,
        b"attacker-controlled bytes\n",
        &log,
        &mut failures,
        &mut evidence,
    )?;
    finish(CASES[1], evidence, failures, &dir)
}

/// A3: a canonical file with the executable bit set syncs, but the
/// bit is stripped on write — the target file has zero exec bits.
fn case_exec_bit_stripped() -> Result<CaseReport, TaskDriverError> {
    let dir = scratch_dir(CASES[2]);
    let _ = fs::remove_dir_all(&dir);
    let canonical = dir.join("canonical");
    let target = dir.join("target");
    let backup = dir.join("backup");
    for d in [&canonical, &target] {
        fs::create_dir_all(d).map_err(|e| fixture("mkdir", e.to_string()))?;
    }
    let src = canonical.join("runme.sh");
    fs::write(&src, b"#!/bin/sh\necho hi\n").map_err(|e| fixture("write", e.to_string()))?;
    let mut perms = fs::metadata(&src)
        .map_err(|e| fixture("metadata", e.to_string()))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&src, perms).map_err(|e| fixture("chmod", e.to_string()))?;

    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut log: Vec<FsAccess> = Vec::new();
    let report = crate::skill_sync::sync(&canonical, &target, &ApplyOptions::new(backup), &mut log)
        .map_err(|e| fixture("sync", e.to_string()))?;
    if report.applied.len() != 1 {
        failures.push(format!("applied {} ops, want 1 add", report.applied.len()));
    }
    if report.dry_run {
        failures.push("non-dry-run reported dry_run".to_string());
    }
    let dest = target.join("runme.sh");
    let mode = fs::metadata(&dest)
        .map_err(|e| fixture("metadata", e.to_string()))?
        .permissions()
        .mode();
    // Mask to the permission bits: mode() carries the file-type bits
    // too (0o100000 for a regular file).
    evidence.push(format!("target runme.sh mode: {:o}", mode & 0o777));
    if mode & 0o111 != 0 {
        failures.push(format!("exec bits survived: mode {mode:o}"));
    }
    if mode & 0o777 != 0o644 {
        failures.push(format!("mode is {mode:o}, want exactly 644"));
    }
    let bytes = fs::read(&dest).map_err(|e| fixture("read", e.to_string()))?;
    if bytes != b"#!/bin/sh\necho hi\n" {
        failures.push("content changed while stripping bits".to_string());
    }
    finish(CASES[2], evidence, failures, &dir)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "traversal_refused" => case_traversal_refused(),
        "symlink_not_followed" => case_symlink_not_followed(),
        "exec_bit_stripped" => case_exec_bit_stripped(),
        _ => Err(fixture("case", format!("unknown case '{case}'"))),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// traversal refusal.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-195".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-195".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
