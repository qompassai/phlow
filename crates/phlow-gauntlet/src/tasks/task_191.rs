// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 191 — query literal handling (rust, A).
//!
//! The user query is data, not code. LIKE wildcards (`%`, `_`) in the
//! query are escaped and matched literally, and SQL operators typed
//! into the query box can never widen the match: the query travels to
//! SQLite only as bound parameters inside LIKE patterns with
//! `ESCAPE '\'`. The query log records the escaped form as evidence.
//!
//! Primary sources: Ghostex `packages/find` engine (the adapted seam);
//! SQLite LIKE documentation (sqlite.org/lang_expr.html — `%`, `_`
//! wildcards and the ESCAPE clause); the escaping contract is ours.

use std::path::PathBuf;

use crate::session_find::{
    DB_FILE_NAME, FsLog, SessionIndex, SessionSpec, SqlLog, escape_like, fresh_temp_dir,
    scan_and_index, write_session,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-191";
/// Task name.
pub const NAME: &str = "query literal handling";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 adversarial.
pub const CASES: [&str; 2] = [
    "percent_matches_literally",
    "or_injection_matches_literally",
];
/// The wildcard query: must match only literal `%` sessions.
pub const PERCENT_QUERY: &str = "%";
/// The injection-shaped query: must match nothing (no such literal).
pub const INJECTION_QUERY: &str = "\" OR \"1\"=\"1";

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

fn build_corpus(tag: &str) -> (PathBuf, PathBuf) {
    let tmp = fresh_temp_dir(tag);
    let sessions = tmp.join("sessions");
    std::fs::create_dir_all(&sessions).expect("session dir must be creatable");
    let specs = [
        (
            "pct",
            "100% coverage report",
            "coverage reached one hundred percent",
        ),
        ("quoted", "he said \"hi\" loudly", "a session about quoting"),
        (
            "plain-a",
            "refactor the parser",
            "routine engineering notes",
        ),
        ("plain-b", "flaky test triage", "more routine notes"),
        ("plain-c", "release checklist", "still routine notes"),
        ("plain-d", "oncall handoff", "yet more routine notes"),
    ];
    for (stem, title, transcript) in specs {
        write_session(&sessions, stem, &SessionSpec::new(stem, title, transcript));
    }
    let db = tmp.join("index").join(DB_FILE_NAME);
    std::fs::create_dir_all(db.parent().unwrap()).expect("index dir must be creatable");
    let mut fs_log = FsLog::new();
    let mut sql_log = SqlLog::new();
    scan_and_index(&sessions, &db, &mut fs_log, &mut sql_log).expect("scan must succeed");
    (tmp, db)
}

/// A1: query `%` → matches only sessions containing a literal `%`,
/// not everything. The query log shows the escaped LIKE form.
fn case_percent_matches_literally() -> Result<CaseReport, TaskDriverError> {
    let (_tmp, db) = build_corpus("t191-a1");
    let mut index = SessionIndex::open_strict(&db).map_err(|e| arm_error("open", e.to_string()))?;
    let total = index
        .count()
        .map_err(|e| arm_error("count", e.to_string()))?;
    let hits = index
        .search(PERCENT_QUERY, 50)
        .map_err(|e| arm_error("search", e.to_string()))?;
    let mut failures = Vec::new();
    if hits.is_empty() {
        failures.push("query '%' matched nothing; the literal-% session must match".to_string());
    }
    if hits.len() >= total {
        failures.push(format!(
            "query '%' matched {}/{} sessions: wildcard was not escaped",
            hits.len(),
            total
        ));
    }
    for hit in &hits {
        if !hit.title.contains('%') {
            failures.push(format!(
                "hit '{}' contains no literal '%': wildcard leaked",
                hit.id
            ));
        }
    }
    if !hits.iter().any(|h| h.id == "pct") {
        failures.push("the '100% coverage report' session is missing from '%' hits".to_string());
    }
    let log = index.query_log().join("\n");
    if !log.contains(r"\%") {
        failures.push(format!("query log lacks the escaped form: {log}"));
    }
    let evidence = vec![
        format!(
            "query '%' → {}/{} hits, all containing a literal '%': {}",
            hits.len(),
            total,
            hits.iter()
                .map(|h| h.id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        format!("escaped query log: {log}"),
    ];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "query": PERCENT_QUERY,
            "hits": hits.len(),
            "total": total,
            "escaped": log.contains(r"\%"),
            "backend": "sqlite",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// A2: query `" OR "1"="1` → zero matches (no session contains that
/// literal); the table is untouched; the log shows the escaped form.
fn case_or_injection_matches_literally() -> Result<CaseReport, TaskDriverError> {
    let (_tmp, db) = build_corpus("t191-a2");
    let mut index = SessionIndex::open_strict(&db).map_err(|e| arm_error("open", e.to_string()))?;
    let before = index
        .count()
        .map_err(|e| arm_error("count", e.to_string()))?;
    let hits = index
        .search(INJECTION_QUERY, 50)
        .map_err(|e| arm_error("search", e.to_string()))?;
    let after = index
        .count()
        .map_err(|e| arm_error("count", e.to_string()))?;
    let mut failures = Vec::new();
    if !hits.is_empty() {
        failures.push(format!(
            "injection-shaped query matched {} session(s): the query widened",
            hits.len()
        ));
    }
    if after != before {
        failures.push(format!("table changed by a query: {before} → {after} rows"));
    }
    let log = index.query_log().join("\n");
    // The query is tokenized on whitespace, so the raw contiguous
    // string cannot appear; the escaped LIKE pattern of the
    // injection fragment must appear literally instead.
    // The query log records the pattern list with Debug formatting,
    // which escapes the quotes; assert on that exact representation.
    let escaped_fragment = format!("{:?}", format!("%{}%", escape_like("\"1\"=\"1")));
    if !log.contains(&escaped_fragment) {
        failures.push(format!(
            "query log does not show the escaped query form: {log}"
        ));
    }
    let evidence = vec![
        format!(
            "query '{INJECTION_QUERY}' → {} hits (want 0); rows before={before} after={after}",
            hits.len(),
        ),
        format!("escaped query log: {log}"),
    ];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "query": INJECTION_QUERY,
            "hits": hits.len(),
            "rows_before": before,
            "rows_after": after,
            "backend": "sqlite",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "percent_matches_literally" => case_percent_matches_literally(),
        "or_injection_matches_literally" => case_or_injection_matches_literally(),
        _ => Err(arm_error(
            "case",
            format!("task-191: unknown case '{case}'"),
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
            where_: "task-191".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-191".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
