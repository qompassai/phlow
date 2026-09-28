// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 190 — corrupt index rebuild (rust, A).
//!
//! The index file is hostile input too. A zeroed header or a
//! mid-table truncation must surface as typed [`IndexError::Corrupt`]
//! (detected by `PRAGMA integrity_check`), never a panic and never
//! partial answers. [`open_or_rebuild`] quarantines the corrupt file
//! and rebuilds from a fresh scan; before recovery, opening fails
//! closed — no query is ever served from the corrupt file.
//!
//! Primary sources: SQLite `PRAGMA integrity_check` documentation
//! (sqlite.org/pragma.html#pragma_integrity_check); the
//! rebuild-from-scan contract is ours.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::session_find::{
    DB_FILE_NAME, FsLog, IndexError, SessionIndex, SessionSpec, SqlLog, fresh_temp_dir,
    open_or_rebuild, scan_and_index, write_session,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-190";
/// Task name.
pub const NAME: &str = "corrupt index rebuild";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = ["zeroed_header_rebuilds", "truncated_midtable_rebuilds"];
/// Sessions in the rebuild corpus.
pub const CORPUS_SIZE: usize = 20;
/// Bytes zeroed at the start of the index file (destroys the header).
pub const ZERO_BYTES: usize = 4096;

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

fn build_corpus(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let tmp = fresh_temp_dir(tag);
    let sessions = tmp.join("sessions");
    std::fs::create_dir_all(&sessions).expect("session dir must be creatable");
    for i in 0..CORPUS_SIZE {
        let spec = SessionSpec::new(
            &format!("rebuild-{i:02}"),
            &format!("rebuildable session {i:02}"),
            "stable content",
        );
        write_session(&sessions, &format!("r{i:02}"), &spec);
    }
    let db = tmp.join("index").join(DB_FILE_NAME);
    std::fs::create_dir_all(db.parent().unwrap()).expect("index dir must be creatable");
    (tmp, sessions, db)
}

fn scan(sessions: &Path, db: &Path) {
    let mut fs_log = FsLog::new();
    let mut sql_log = SqlLog::new();
    scan_and_index(sessions, db, &mut fs_log, &mut sql_log).expect("clean scan must succeed");
}

/// Remove WAL sidecars so a corrupted main file cannot be silently
/// repaired by replaying the intact write-ahead log — real
/// corruption takes the whole file family with it.
fn drop_sidecars(db: &Path) {
    for suffix in ["-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
}

/// Corrupt the file, then prove: strict open fails typed-Corrupt (no
/// partial data possible), recovery rebuilds, queries work after.
fn corrupt_and_recover(
    tag: &str,
    corrupt: impl FnOnce(&Path),
    case: &'static str,
) -> Result<CaseReport, TaskDriverError> {
    let (_tmp, sessions, db) = build_corpus(tag);
    let t0 = Instant::now();
    scan(&sessions, &db);
    let clean_scan = t0.elapsed();
    corrupt(&db);
    let mut failures = Vec::new();
    check_fails_closed(&db, &mut failures);
    let mut fs_log = FsLog::new();
    let mut sql_log = SqlLog::new();
    let t1 = Instant::now();
    let recovered = open_or_rebuild(&sessions, &db, &mut fs_log, &mut sql_log)
        .map_err(|e| arm_error("recover", e.to_string()))?;
    let rebuild_scan = t1.elapsed();
    if !recovered.rebuilt {
        failures.push("corrupt index was not detected as needing rebuild".to_string());
    }
    if recovered.quarantined.as_ref().is_none_or(|p| !p.exists()) {
        failures.push("corrupt file was not quarantined aside".to_string());
    }
    let mut index = recovered.index;
    let count = index
        .count()
        .map_err(|e| arm_error("count", e.to_string()))?;
    if count != CORPUS_SIZE {
        failures.push(format!("rows after rebuild={count}, want {CORPUS_SIZE}"));
    }
    let found = check_all_queryable_after_rebuild(&mut index, &mut failures)?;
    // Rebuild must stay within the normal scan bound: no worse than
    // 10× the clean scan of the same corpus.
    if rebuild_scan > clean_scan * 10 {
        failures.push(format!(
            "rebuild took {rebuild_scan:?}, clean scan {clean_scan:?} (bound: 10×)"
        ));
    }
    let evidence = vec![
        format!(
            "corruption detected as IndexError::Corrupt; quarantined to {:?}; rebuilt, rows={count}, queryable={found}/{CORPUS_SIZE}",
            recovered.quarantined,
        ),
        format!(
            "rebuild scanned {} files in {rebuild_scan:?} vs clean scan {clean_scan:?} (bound 10×)",
            recovered.stats.as_ref().map(|s| s.files_read).unwrap_or(0),
        ),
    ];
    let mut report = CaseReport::pass(
        case,
        serde_json::json!({
            "rebuilt": recovered.rebuilt,
            "quarantined": recovered.quarantined.is_some(),
            "rows": count,
            "queryable": found,
            "backend": "sqlite",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Before recovery: the corrupt file fails closed with a typed
/// error. No query handle can exist, so no partial data is served.
fn check_fails_closed(db: &Path, failures: &mut Vec<String>) {
    match SessionIndex::open_strict(db) {
        Err(IndexError::Corrupt(_)) => {}
        Err(other) => failures.push(format!("open gave {other:?}, want IndexError::Corrupt")),
        Ok(_) => failures.push("corrupt index opened without error".to_string()),
    }
}

/// After rebuild: every session queryable by its token.
fn check_all_queryable_after_rebuild(
    index: &mut SessionIndex,
    failures: &mut Vec<String>,
) -> Result<usize, TaskDriverError> {
    let mut found = 0usize;
    for i in 0..CORPUS_SIZE {
        let hits = index
            .search(&format!("rebuildable session {i:02}"), 3)
            .map_err(|e| arm_error("search", e.to_string()))?;
        if hits
            .first()
            .is_some_and(|h| h.id == format!("rebuild-{i:02}"))
        {
            found += 1;
        }
    }
    if found != CORPUS_SIZE {
        failures.push(format!(
            "{found}/{CORPUS_SIZE} sessions queryable after rebuild"
        ));
    }
    Ok(found)
}

/// A1: zero the first 4 KiB (SQLite header + first page) → Corrupt →
/// quarantine + rebuild → queries work.
fn case_zeroed_header_rebuilds() -> Result<CaseReport, TaskDriverError> {
    corrupt_and_recover(
        "t190-a1",
        |db| {
            let mut bytes = std::fs::read(db).expect("index must exist");
            for b in bytes.iter_mut().take(ZERO_BYTES) {
                *b = 0;
            }
            std::fs::write(db, &bytes).expect("corruption must be writable");
            drop_sidecars(db);
        },
        CASES[0],
    )
}

/// A2: truncate the file mid-table → same typed path, same bound.
fn case_truncated_midtable_rebuilds() -> Result<CaseReport, TaskDriverError> {
    corrupt_and_recover(
        "t190-a2",
        |db| {
            let len = std::fs::metadata(db).expect("index must exist").len();
            // Cut at 3/7 of the length: inside table pages, not on a
            // page boundary, so the header page count lies.
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(db)
                .expect("index must open");
            file.set_len(len * 3 / 7).expect("truncate must succeed");
            drop_sidecars(db);
        },
        CASES[1],
    )
}

/// Run one driver case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "zeroed_header_rebuilds" => case_zeroed_header_rebuilds(),
        "truncated_midtable_rebuilds" => case_truncated_midtable_rebuilds(),
        _ => Err(arm_error(
            "case",
            format!("task-190: unknown case '{case}'"),
        )),
    }
}

/// Task-level entry for the gauntlet runner: the headline case.
pub fn run(_ctx: &crate::Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-190".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-190".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
