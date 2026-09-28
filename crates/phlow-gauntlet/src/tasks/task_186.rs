// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 186 — scan-once SQLite index (rust, V).
//!
//! The session dir is scanned once into SQLite; a rescan is
//! incremental, keyed on (mtime, size) per file. Unchanged files are
//! never re-read — proven by the [`FsLog`] read counter, not by trust.
//!
//! The driver uses the scripted doubles: [`FsLog`] (MOCK) recording
//! every session-file read, and the synthetic session corpus (MOCK)
//! with one unique token per session title.
//!
//! Primary sources: Ghostex `packages/find/src/index.rs` "scan once"
//! design (read-only clone at `~/workspace/scratch/ghostex/` @
//! c911466); the (mtime, size) incremental key is ours.

use std::path::{Path, PathBuf};

use crate::session_find::{
    DB_FILE_NAME, FsLog, SessionIndex, SessionSpec, SqlLog, fresh_temp_dir, scan_and_index,
    write_session,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-186";
/// Task name.
pub const NAME: &str = "scan-once SQLite index";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["full_build_queryable", "incremental_rescan_four_reads"];
/// Number of scripted session files in the full-build corpus.
pub const CORPUS_SIZE: usize = 500;

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

fn session_dir(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let tmp = fresh_temp_dir(tag);
    let sessions = tmp.join("sessions");
    std::fs::create_dir_all(&sessions).expect("session dir must be creatable");
    let db = tmp.join("index").join(DB_FILE_NAME);
    std::fs::create_dir_all(db.parent().unwrap()).expect("index dir must be creatable");
    (tmp, sessions, db)
}

/// Write the 500-session corpus. Each title carries a unique token so
/// every session is individually queryable.
fn write_corpus(sessions: &Path, count: usize) {
    for i in 0..count {
        let id = format!("sess-{i:04}");
        let spec = SessionSpec::new(
            &id,
            &format!("session {i:04} token-{i:04} work notes"),
            &format!("transcript batch {i}"),
        );
        write_session(sessions, &format!("s{i:04}"), &spec);
    }
}

fn file_mtime_secs(path: &Path) -> i64 {
    use std::time::UNIX_EPOCH;
    std::fs::metadata(path)
        .expect("fixture must exist")
        .modified()
        .expect("mtime must exist")
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// V1: 500 session files → index built, all 500 queryable by their
/// unique token, recorded mtimes match the files.
fn case_full_build_queryable() -> Result<CaseReport, TaskDriverError> {
    let (_tmp, sessions, db) = session_dir("t186-v1");
    write_corpus(&sessions, CORPUS_SIZE);
    let mut fs_log = FsLog::new();
    let mut sql_log = SqlLog::new();
    let stats = scan_and_index(&sessions, &db, &mut fs_log, &mut sql_log)
        .map_err(|e| arm_error("scan", e.to_string()))?;
    let mut failures = Vec::new();
    if stats.files_read != CORPUS_SIZE {
        failures.push(format!(
            "files_read={}, want {CORPUS_SIZE}",
            stats.files_read
        ));
    }
    if fs_log.read_count() != CORPUS_SIZE {
        failures.push(format!(
            "fs reads={}, want {CORPUS_SIZE}",
            fs_log.read_count()
        ));
    }
    let mut index = SessionIndex::open_strict(&db).map_err(|e| arm_error("open", e.to_string()))?;
    let count = index
        .count()
        .map_err(|e| arm_error("count", e.to_string()))?;
    if count != CORPUS_SIZE {
        failures.push(format!("indexed rows={count}, want {CORPUS_SIZE}"));
    }
    let queried_ok = check_all_queryable(&mut index, &mut failures)?;
    let mtime_ok = check_mtime_sample(&mut index, &sessions, &mut failures)?;
    let evidence = vec![
        format!(
            "full build: files_read={} fs_reads={} rows={} queried_ok={queried_ok}/{CORPUS_SIZE} mtime_ok={mtime_ok}/5",
            stats.files_read,
            fs_log.read_count(),
            count,
        ),
        format!("generation stamped: {}", stats.generation),
    ];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "files_read": stats.files_read,
            "rows": count,
            "queried_ok": queried_ok,
            "mtime_ok": mtime_ok,
            "backend": "sqlite",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Every session queryable by its unique token, as the top hit.
/// Stops at the first miss (the failure names it).
fn check_all_queryable(
    index: &mut SessionIndex,
    failures: &mut Vec<String>,
) -> Result<usize, TaskDriverError> {
    let mut queried_ok = 0usize;
    for i in 0..CORPUS_SIZE {
        let hits = index
            .search(&format!("token-{i:04}"), 3)
            .map_err(|e| arm_error("search", e.to_string()))?;
        let want = format!("sess-{i:04}");
        if hits.first().is_some_and(|h| h.id == want) {
            queried_ok += 1;
        } else {
            failures.push(format!("token-{i:04} did not rank {want} first"));
            break;
        }
    }
    Ok(queried_ok)
}

/// Recorded mtimes match the files for a 5-file sample.
fn check_mtime_sample(
    index: &mut SessionIndex,
    sessions: &Path,
    failures: &mut Vec<String>,
) -> Result<usize, TaskDriverError> {
    let mut mtime_ok = 0usize;
    for i in [0, 123, 249, 374, 499] {
        let rel = format!("s{i:04}.session.json");
        let recorded = index
            .row_mtime(&rel)
            .map_err(|e| arm_error("row_mtime", e.to_string()))?;
        let actual = file_mtime_secs(&sessions.join(&rel));
        if recorded == Some(actual) {
            mtime_ok += 1;
        } else {
            failures.push(format!(
                "mtime mismatch for {rel}: {recorded:?} vs {actual}"
            ));
        }
    }
    Ok(mtime_ok)
}

/// Change the world between scans: 3 new files, 1 modified (new
/// bytes → new size, so the (mtime, size) key trips even on coarse
/// mtimes).
fn add_and_modify(sessions: &Path) {
    for i in 0..3 {
        let id = format!("sess-new-{i}");
        let spec = SessionSpec::new(&id, &format!("brand new session token-new-{i}"), "new");
        write_session(sessions, &format!("new{i}"), &spec);
    }
    let modified = SessionSpec::new(
        "sess-0000",
        "session 0000 token-0000 UPDATED with more words here",
        "changed transcript",
    );
    write_session(sessions, "s0000", &modified);
}

/// V2: add 3 files, modify 1 → rescan reads exactly those 4 files;
/// the fs-access log names them; everything else is queryable.
fn case_incremental_rescan_four_reads() -> Result<CaseReport, TaskDriverError> {
    let (_tmp, sessions, db) = session_dir("t186-v2");
    write_corpus(&sessions, CORPUS_SIZE);
    let mut fs_log = FsLog::new();
    let mut sql_log = SqlLog::new();
    scan_and_index(&sessions, &db, &mut fs_log, &mut sql_log)
        .map_err(|e| arm_error("scan", e.to_string()))?;
    add_and_modify(&sessions);
    fs_log.clear();
    let stats = scan_and_index(&sessions, &db, &mut fs_log, &mut sql_log)
        .map_err(|e| arm_error("rescan", e.to_string()))?;
    let mut failures = Vec::new();
    if stats.files_read != 4 {
        failures.push(format!("rescan files_read={}, want 4", stats.files_read));
    }
    if fs_log.read_count() != 4 {
        failures.push(format!(
            "rescan fs reads={}, want exactly 4",
            fs_log.read_count()
        ));
    }
    let names = sorted_read_names(&fs_log);
    let want = vec![
        "new0.session.json",
        "new1.session.json",
        "new2.session.json",
        "s0000.session.json",
    ];
    if names != want {
        failures.push(format!("rescan read files {names:?}, want {want:?}"));
    }
    let mut index = SessionIndex::open_strict(&db).map_err(|e| arm_error("open", e.to_string()))?;
    let count = index
        .count()
        .map_err(|e| arm_error("count", e.to_string()))?;
    if count != CORPUS_SIZE + 3 {
        failures.push(format!(
            "rows after rescan={count}, want {}",
            CORPUS_SIZE + 3
        ));
    }
    check_changed_queryable(&mut index, &mut failures)?;
    let evidence = vec![
        format!(
            "rescan: files_read={} fs_reads={} (want 4/4); rows={count}; new+modified queryable",
            stats.files_read,
            fs_log.read_count(),
        ),
        format!("rescan touched exactly: {names:?}"),
    ];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "rescan_files_read": stats.files_read,
            "rescan_fs_reads": fs_log.read_count(),
            "rows": count,
            "backend": "sqlite",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// File names read during the rescan, sorted, for exact-set comparison.
fn sorted_read_names(fs_log: &FsLog) -> Vec<String> {
    let mut names: Vec<String> = fs_log
        .reads()
        .iter()
        .map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    names.sort();
    names
}

/// The 3 new sessions and the 1 modified session are queryable with
/// their new content after the rescan.
fn check_changed_queryable(
    index: &mut SessionIndex,
    failures: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let hits = index
        .search("token-new-1", 3)
        .map_err(|e| arm_error("search", e.to_string()))?;
    if !hits.first().is_some_and(|h| h.id == "sess-new-1") {
        failures.push("new session sess-new-1 not queryable after rescan".to_string());
    }
    let hits = index
        .search("UPDATED", 3)
        .map_err(|e| arm_error("search", e.to_string()))?;
    if !hits.first().is_some_and(|h| h.id == "sess-0000") {
        failures.push("modified session sess-0000 not re-indexed".to_string());
    }
    Ok(())
}

/// Run one driver case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "full_build_queryable" => case_full_build_queryable(),
        "incremental_rescan_four_reads" => case_incremental_rescan_four_reads(),
        _ => Err(arm_error(
            "case",
            format!("task-186: unknown case '{case}'"),
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
            where_: "task-186".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-186".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
