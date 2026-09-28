// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 189 — poisoned session index (rust, A).
//!
//! Session files are untrusted input. SQL metacharacters in every
//! text field must land in the database literally (bound parameters,
//! proven by the [`SqlLog`] template audit — a test double that
//! records every statement template and rejects any raw-SQL
//! interpolation), and a 20 MB field must truncate at
//! [`MAX_FIELD_BYTES`] with the truncation flag set while the build
//! completes.
//!
//! Primary sources: Ghostex `packages/find` engine (the adapted seam);
//! SQL parameterization practice (OWASP SQL Injection Prevention:
//! parameterized queries keep code and data separate).

use std::path::PathBuf;

use crate::session_find::{
    DB_FILE_NAME, FsLog, MAX_FIELD_BYTES, SessionIndex, SessionSpec, SqlLog, fresh_temp_dir,
    scan_and_index, write_session,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-189";
/// Task name.
pub const NAME: &str = "poisoned session index";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = [
    "sql_metacharacters_stored_literally",
    "huge_field_truncated",
];
/// The hostile payload, placed in every text field.
pub const POISON: &str = "'); DROP TABLE sessions; --";
/// The oversized-field payload size (20 MiB).
pub const HUGE_BYTES: usize = 20 * 1_048_576;

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

fn index_paths(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let tmp = fresh_temp_dir(tag);
    let sessions = tmp.join("sessions");
    std::fs::create_dir_all(&sessions).expect("session dir must be creatable");
    let db = tmp.join("index").join(DB_FILE_NAME);
    std::fs::create_dir_all(db.parent().unwrap()).expect("index dir must be creatable");
    (tmp, sessions, db)
}

/// A1: the poison string in every text field → stored literally, the
/// table intact and queryable, and the SqlLog proves every write used
/// the single fixed parameterized template (no interpolated SQL).
fn case_sql_metacharacters_stored_literally() -> Result<CaseReport, TaskDriverError> {
    let (_tmp, sessions, db) = index_paths("t189-a1");
    let spec = SessionSpec {
        id: POISON.to_string(),
        title: POISON.to_string(),
        project: POISON.to_string(),
        transcript: POISON.to_string(),
        started_at: 1_700_000_001,
    };
    write_session(&sessions, "poison", &spec);
    let mut fs_log = FsLog::new();
    let mut sql_log = SqlLog::new();
    scan_and_index(&sessions, &db, &mut fs_log, &mut sql_log)
        .map_err(|e| arm_error("scan", e.to_string()))?;
    let mut failures = Vec::new();
    let write_templates = check_no_interpolation(&sql_log, &mut failures);
    let mut index = SessionIndex::open_strict(&db).map_err(|e| arm_error("open", e.to_string()))?;
    let count = index
        .count()
        .map_err(|e| arm_error("count", e.to_string()))?;
    if count != 1 {
        failures.push(format!("row count={count}, want 1 (table must be intact)"));
    }
    let title = index
        .row_title("poison.session.json")
        .map_err(|e| arm_error("row_title", e.to_string()))?;
    if title.as_deref() != Some(POISON) {
        failures.push("poison title was not stored literally".to_string());
    }
    check_poison_queryable(&mut index, &mut failures)?;
    let evidence = vec![
        format!(
            "poison '{POISON}' in every field: rows={count}, title stored literally, queryable"
        ),
        format!("write templates used: {write_templates:?}"),
    ];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "rows": count,
            "literal": title.as_deref() == Some(POISON),
            "write_templates": write_templates.len(),
            "backend": "sqlite",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Audit the recorded statement templates: none may contain the
/// payload, and exactly one session-write template may exist.
/// Returns the distinct write templates for the report.
fn check_no_interpolation(sql_log: &SqlLog, failures: &mut Vec<String>) -> Vec<String> {
    let mut templates: Vec<String> = sql_log.statements().to_vec();
    templates.sort();
    templates.dedup();
    for template in &templates {
        if template.contains("DROP TABLE") || template.contains(POISON) {
            failures.push(format!("payload interpolated into SQL: {template}"));
        }
    }
    let write_templates: Vec<String> = templates
        .iter()
        .filter(|t| t.starts_with("INSERT INTO sessions"))
        .cloned()
        .collect();
    if write_templates.len() != 1 {
        failures.push(format!(
            "want exactly 1 session-write template, saw {}: {write_templates:?}",
            write_templates.len()
        ));
    }
    write_templates
}

/// The poison is searchable as data: querying a distinctive token of
/// it finds the row through the literal LIKE path.
fn check_poison_queryable(
    index: &mut SessionIndex,
    failures: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let hits = index
        .search("DROP TABLE sessions", 5)
        .map_err(|e| arm_error("search", e.to_string()))?;
    if !hits.iter().any(|h| h.title == POISON) {
        failures.push("poisoned row not queryable as literal data".to_string());
    }
    Ok(())
}

/// A2: a 20 MiB single field → truncated at MAX_FIELD_BYTES with the
/// truncation flag set; the build completes and the index is usable.
fn case_huge_field_truncated() -> Result<CaseReport, TaskDriverError> {
    let (_tmp, sessions, db) = index_paths("t189-a2");
    let big = "x".repeat(HUGE_BYTES);
    let spec = SessionSpec::new("huge", "huge session", &big);
    write_session(&sessions, "huge", &spec);
    let mut fs_log = FsLog::new();
    let mut sql_log = SqlLog::new();
    let stats = scan_and_index(&sessions, &db, &mut fs_log, &mut sql_log)
        .map_err(|e| arm_error("scan", e.to_string()))?;
    let mut failures = Vec::new();
    let index = SessionIndex::open_strict(&db).map_err(|e| arm_error("open", e.to_string()))?;
    let info = index
        .row_info("huge.session.json")
        .map_err(|e| arm_error("row_info", e.to_string()))?;
    match info {
        Some((len, truncated)) => {
            if len as usize != MAX_FIELD_BYTES {
                failures.push(format!(
                    "stored transcript len={len}, want exactly {MAX_FIELD_BYTES}"
                ));
            }
            if !truncated {
                failures.push("truncation flag not set on the 20 MiB field".to_string());
            }
        }
        None => failures.push("huge session row missing".to_string()),
    }
    let count = index
        .count()
        .map_err(|e| arm_error("count", e.to_string()))?;
    if count != 1 || stats.rows_upserted != 1 {
        failures.push(format!(
            "build did not complete cleanly: rows={count} upserted={}",
            stats.rows_upserted
        ));
    }
    let evidence = vec![format!(
        "20 MiB field: stored {} bytes (bound {MAX_FIELD_BYTES}), truncated flag set, build completed",
        info.map(|(l, _)| l).unwrap_or(-1),
    )];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "input_bytes": HUGE_BYTES,
            "stored_bytes": info.map(|(l, _)| l).unwrap_or(-1),
            "truncated": info.map(|(_, t)| t).unwrap_or(false),
            "bound": MAX_FIELD_BYTES,
            "backend": "sqlite",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "sql_metacharacters_stored_literally" => case_sql_metacharacters_stored_literally(),
        "huge_field_truncated" => case_huge_field_truncated(),
        _ => Err(arm_error(
            "case",
            format!("task-189: unknown case '{case}'"),
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
            where_: "task-189".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-189".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
