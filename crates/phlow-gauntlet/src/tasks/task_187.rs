// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 187 — ranked fuzzy queries (rust, V).
//!
//! Fuzzy search returns deterministically ranked hits: the session
//! whose title carries the query words is rank 1, and the ranking is
//! byte-identical across runs (total order on (score desc, id asc) —
//! no nondeterministic tie-breaking anywhere).
//!
//! The driver uses the scripted doubles: [`FsLog`] (MOCK) and the
//! shared 200-session `pairing_corpus` (MOCK) with exactly one
//! three-word title match.
//!
//! Primary sources: Ghostex `packages/find/src/index.rs`
//! (`hit_less` total ordering) and `packages/find/src/fuzzy.rs`
//! (fzf-style matcher); the scoring function here is ours.

use std::path::PathBuf;

use crate::session_find::{
    DB_FILE_NAME, FsLog, SessionIndex, SqlLog, fresh_temp_dir, pairing_corpus, scan_and_index,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-187";
/// Task name.
pub const NAME: &str = "ranked fuzzy queries";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["target_ranks_first", "rankings_byte_identical"];
/// The query both cases run.
pub const QUERY: &str = "pairing brute force";
/// How many times the determinism case repeats the query.
pub const REPEAT: usize = 10;

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

fn build_index(tag: &str) -> Result<(PathBuf, PathBuf, String), TaskDriverError> {
    let tmp = fresh_temp_dir(tag);
    let sessions = tmp.join("sessions");
    std::fs::create_dir_all(&sessions).expect("session dir must be creatable");
    let target_id = pairing_corpus(&sessions);
    let db = tmp.join("index").join(DB_FILE_NAME);
    std::fs::create_dir_all(db.parent().unwrap()).expect("index dir must be creatable");
    let mut fs_log = FsLog::new();
    let mut sql_log = SqlLog::new();
    scan_and_index(&sessions, &db, &mut fs_log, &mut sql_log)
        .map_err(|e| arm_error("scan", e.to_string()))?;
    Ok((tmp, db, target_id))
}

fn ranking_signature(index: &mut SessionIndex) -> Result<String, TaskDriverError> {
    let hits = index
        .search(QUERY, 25)
        .map_err(|e| arm_error("search", e.to_string()))?;
    Ok(format!(
        "{:?}",
        hits.iter()
            .map(|h| (h.id.clone(), h.score))
            .collect::<Vec<_>>()
    ))
}

/// V1: 200 indexed sessions, query "pairing brute force" → the
/// session titled with those words is rank 1.
fn case_target_ranks_first() -> Result<CaseReport, TaskDriverError> {
    let (_tmp, db, target_id) = build_index("t187-v1")?;
    let mut index = SessionIndex::open_strict(&db).map_err(|e| arm_error("open", e.to_string()))?;
    let hits = index
        .search(QUERY, 25)
        .map_err(|e| arm_error("search", e.to_string()))?;
    let mut failures = Vec::new();
    match hits.first() {
        Some(top) if top.id == target_id => {}
        Some(top) => failures.push(format!(
            "rank 1 is '{}' (score {}), want '{target_id}'",
            top.id, top.score
        )),
        None => failures.push("query returned zero hits".to_string()),
    }
    if hits.len() < 2 {
        failures.push(format!(
            "only {} hit(s); the ranking needs distractors to be meaningful",
            hits.len()
        ));
    }
    let top3: Vec<String> = hits
        .iter()
        .take(3)
        .map(|h| format!("{}:{}", h.id, h.score))
        .collect();
    let evidence = vec![
        format!(
            "query '{QUERY}' over 200 sessions: rank 1 = '{}'",
            target_id
        ),
        format!("top 3 (id:score): {}", top3.join(", ")),
        format!("total hits: {}", hits.len()),
    ];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "rank_1": hits.first().map(|h| h.id.clone()),
            "rank_1_score": hits.first().map(|h| h.score),
            "hits": hits.len(),
            "backend": "sqlite",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: the same query 10× → byte-identical rankings every time.
fn case_rankings_byte_identical() -> Result<CaseReport, TaskDriverError> {
    let (_tmp, db, _target) = build_index("t187-v2")?;
    let mut index = SessionIndex::open_strict(&db).map_err(|e| arm_error("open", e.to_string()))?;
    let mut failures = Vec::new();
    let first = ranking_signature(&mut index)?;
    let mut identical = 1usize;
    for run in 1..REPEAT {
        let sig = ranking_signature(&mut index)?;
        if sig == first {
            identical += 1;
        } else {
            failures.push(format!("run {run}: ranking differs from run 0"));
        }
    }
    let evidence = vec![
        format!("'{QUERY}' × {REPEAT}: {identical}/{REPEAT} byte-identical rankings"),
        format!("signature bytes: {}", first.len()),
    ];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "runs": REPEAT,
            "identical": identical,
            "signature_len": first.len(),
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
        "target_ranks_first" => case_target_ranks_first(),
        "rankings_byte_identical" => case_rankings_byte_identical(),
        _ => Err(arm_error(
            "case",
            format!("task-187: unknown case '{case}'"),
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
            where_: "task-187".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-187".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
