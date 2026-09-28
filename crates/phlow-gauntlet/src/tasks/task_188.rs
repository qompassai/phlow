// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.
//! Task 188 — no per-agent parsers (rust, V).
//!
//! The adaptation lifts the zehn *engine* (scan once, ranked queries),
//! not Ghostex's per-agent parsers. Two proofs: a static scan of the
//! adapted engine module shows zero references to foreign agent homes,
//! and a fake foreign-agent tree dropped next to the session dir is
//! never read (the fs-access log proves it).
//!
//! Primary sources: the adaptation map ("SKIP its per-agent parsers",
//! `~/workspace/ghostex-recon/adaptation-map.md`); Ghostex
//! `packages/find/src/agent.rs` @ c911466 (the parsers NOT lifted).

use std::path::PathBuf;

use crate::session_find::{
    DB_FILE_NAME, FsLog, SessionIndex, SessionSpec, SqlLog, fresh_temp_dir, scan_and_index,
    write_session,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-188";
/// Task name.
pub const NAME: &str = "no per-agent parsers";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 2 validation.
pub const CASES: [&str; 2] = ["no_foreign_agent_references", "fake_foreign_tree_ignored"];
/// Foreign-agent markers that must not appear in the adapted engine.
pub const FORBIDDEN_MARKERS: [&str; 5] = [".claude", ".codex", ".gemini", "agent.rs", "Agent::"];

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// V1: static scan of the adapted engine module → zero references to
/// foreign agent homes or Ghostex's per-agent parser module.
fn case_no_foreign_agent_references() -> Result<CaseReport, TaskDriverError> {
    let engine = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/session_find.rs");
    let source = std::fs::read_to_string(&engine)
        .map_err(|e| arm_error("read", format!("{}: {e}", engine.display())))?;
    let mut failures = Vec::new();
    let mut hits = Vec::new();
    for marker in FORBIDDEN_MARKERS {
        let count = source.matches(marker).count();
        if count > 0 {
            hits.push(format!("{marker} × {count}"));
            failures.push(format!(
                "forbidden marker '{marker}' found {count}× in session_find.rs"
            ));
        }
    }
    let evidence = vec![
        format!(
            "static scan of {}: {}",
            engine.display(),
            if hits.is_empty() {
                "zero foreign-agent markers".to_string()
            } else {
                format!("FORBIDDEN: {}", hits.join(", "))
            }
        ),
        format!("markers checked: {}", FORBIDDEN_MARKERS.join(", ")),
    ];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "forbidden_hits": hits,
            "markers_checked": FORBIDDEN_MARKERS.len(),
            "backend": "static-scan",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// V2: a fake foreign-agent tree next to the session dir is ignored —
/// zero reads under it in the fs-access log.
fn case_fake_foreign_tree_ignored() -> Result<CaseReport, TaskDriverError> {
    let tmp = fresh_temp_dir("t188-v2");
    let sessions = tmp.join("sessions");
    std::fs::create_dir_all(&sessions).expect("session dir must be creatable");
    for i in 0..5 {
        let spec = SessionSpec::new(
            &format!("real-{i}"),
            &format!("real session {i}"),
            "phlow's own session history",
        );
        write_session(&sessions, &format!("r{i}"), &spec);
    }
    // The fake foreign tree: plausible agent-history layout, sibling
    // of (not under) the session dir.
    let foreign = tmp.join(".claude").join("projects").join("acme");
    std::fs::create_dir_all(&foreign).expect("fake tree must be creatable");
    for i in 0..10 {
        std::fs::write(
            foreign.join(format!("session-{i}.jsonl")),
            format!("{{\"foreign\":\"agent history {i}\"}}\n"),
        )
        .expect("fake history must be writable");
    }
    let db = tmp.join("index").join(DB_FILE_NAME);
    std::fs::create_dir_all(db.parent().unwrap()).expect("index dir must be creatable");
    let mut fs_log = FsLog::new();
    let mut sql_log = SqlLog::new();
    let stats = scan_and_index(&sessions, &db, &mut fs_log, &mut sql_log)
        .map_err(|e| arm_error("scan", e.to_string()))?;
    let mut failures = Vec::new();
    let foreign_reads = fs_log.reads_under(&tmp.join(".claude"));
    if foreign_reads != 0 {
        failures.push(format!("{foreign_reads} reads under the fake foreign tree"));
    }
    let leaked: Vec<PathBuf> = fs_log
        .reads()
        .iter()
        .filter(|p| p.to_string_lossy().contains(".claude"))
        .cloned()
        .collect();
    if !leaked.is_empty() {
        failures.push(format!("foreign paths read: {leaked:?}"));
    }
    let index = SessionIndex::open_strict(&db).map_err(|e| arm_error("open", e.to_string()))?;
    let count = index
        .count()
        .map_err(|e| arm_error("count", e.to_string()))?;
    if count != 5 {
        failures.push(format!(
            "indexed rows={count}, want 5 (foreign files must not index)"
        ));
    }
    if stats.files_read != 5 {
        failures.push(format!("files_read={}, want 5", stats.files_read));
    }
    let evidence = vec![
        format!(
            "fake foreign tree (10 jsonl files): reads under it = {foreign_reads}; indexed rows = {count}/5"
        ),
        "indexer scanned only the configured session root".to_string(),
    ];
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "foreign_reads": foreign_reads,
            "rows": count,
            "backend": "fs-log",
        }),
        [evidence, failures.clone()].concat(),
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "no_foreign_agent_references" => case_no_foreign_agent_references(),
        "fake_foreign_tree_ignored" => case_fake_foreign_tree_ignored(),
        _ => Err(arm_error(
            "case",
            format!("task-188: unknown case '{case}'"),
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
            where_: "task-188".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-188".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
