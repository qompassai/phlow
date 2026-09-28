// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 152 — unknown fields ignored, never denied (rust, V).
//!
//! The seam is struct deserialization of envelope bodies. Forward
//! compatibility: a peer that adds a field must not break this build,
//! so `deny_unknown_fields` is banned on wire types (Ghostex
//! `packages/gx-protocol/src/lib.rs` rules). The driver parses a
//! field-augmented envelope from a scripted peer (MOCK) and then runs
//! a real static scan over the phlow-mcp and phlow-runtime crate
//! sources asserting zero `deny_unknown_fields` attributes.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::wire::{Kind, parse_envelope};
use crate::{TaskKind, TaskOutcome};
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-152";
/// Task name.
pub const NAME: &str = "unknown fields ignored, never denied";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 3 validation (the 3rd is the license gate).
pub const CASES: [&str; 3] = [
    "unknown_fields_ignored",
    "static_scan_no_deny",
    "license_header_present",
];

fn arm_error(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Arm {
        arm: what.to_string(),
        detail,
    }
}

/// V1: an envelope with 3 known + 5 unknown fields parses; the known
/// fields are exact and the unknowns are dropped (and counted in the
/// debug metric).
fn case_unknown_fields_ignored() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let bytes = br#"{
        "version": 2,
        "kind": "event",
        "id": "e1",
        "future_a": 1,
        "future_b": "x",
        "future_c": null,
        "future_d": [1, 2],
        "future_e": {"k": 1}
    }"#;
    let (env, _) = parse_envelope(bytes).map_err(|e| {
        arm_error(
            "parse",
            format!("task-152: augmented envelope refused: {e:?}"),
        )
    })?;
    if env.version != Some(2) {
        failures.push(format!("version {:?}, want Some(2)", env.version));
    }
    if env.kind != Kind::Event {
        failures.push(format!("kind {:?}, want Event", env.kind));
    }
    if env.id != "e1" {
        failures.push(format!("id {:?}, want \"e1\"", env.id));
    }
    if env.unknown_fields != 5 {
        failures.push(format!(
            "unknown_fields {}, want 5 (debug metric)",
            env.unknown_fields
        ));
    }
    // The dropped fields are gone from the canonical form: the typed
    // value carries no trace of them.
    let canonical = env.to_canonical_json().to_string();
    for dropped in ["future_a", "future_b", "future_c", "future_d", "future_e"] {
        if canonical.contains(dropped) {
            failures.push(format!(
                "dropped field {dropped} leaked into canonical form"
            ));
        }
    }
    let evidence = vec![format!(
        "3 known + 5 unknown fields: parsed, known exact, unknown_fields = {}",
        env.unknown_fields
    )];
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "known_fields": 3,
            "unknown_fields": env.unknown_fields,
            "parsed": true,
            "backend": "scripted-mock",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// Collects every `.rs` file under `root`, iteratively (no
/// recursion). Symlinked directories are not descended into.
fn rs_files_under(root: &Path) -> Result<Vec<PathBuf>, TaskDriverError> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| TaskDriverError::Fixture {
            what: "wire-scan".to_string(),
            detail: format!("task-152: cannot read dir {}: {e}", dir.display()),
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| TaskDriverError::Fixture {
                what: "wire-scan".to_string(),
                detail: format!("task-152: cannot read entry: {e}"),
            })?;
            let path = entry.path();
            let ftype = entry.file_type().map_err(|e| TaskDriverError::Fixture {
                what: "wire-scan".to_string(),
                detail: format!("task-152: cannot stat {}: {e}", path.display()),
            })?;
            if ftype.is_dir() {
                stack.push(path);
            } else if ftype.is_file() && path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// V2: every `.rs` file in the phlow-mcp and phlow-runtime crates is
/// scanned for `deny_unknown_fields`; the count must be zero. The
/// scan fails closed: a crate with zero files found is an error, not
/// a pass.
fn case_static_scan_no_deny() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut total_files = 0usize;
    let mut total_hits = 0usize;
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let roots = [
        manifest.join("../phlow-mcp"),
        manifest.join("../phlow-runtime"),
    ];
    for root in &roots {
        let files = rs_files_under(root)?;
        if files.is_empty() {
            failures.push(format!("no .rs files under {}", root.display()));
            continue;
        }
        let mut hits = 0usize;
        for path in &files {
            let text = std::fs::read_to_string(path).map_err(|e| TaskDriverError::Fixture {
                what: "wire-scan".to_string(),
                detail: format!("task-152: cannot read {}: {e}", path.display()),
            })?;
            hits += text.matches("deny_unknown_fields").count();
        }
        total_files += files.len();
        total_hits += hits;
        evidence.push(format!(
            "{}: {} .rs files, {hits} deny_unknown_fields hits",
            root.display(),
            files.len()
        ));
    }
    if total_hits != 0 {
        failures.push(format!("{total_hits} deny_unknown_fields hits, want 0"));
    }
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "files_scanned": total_files,
            "deny_unknown_fields_hits": total_hits,
            "backend": "static-scan",
        }),
        evidence,
    );
    report.passed = failures.is_empty();
    Ok(report)
}

/// License gate: the adapted wire module carries the maddada
/// attribution and the source commit.
fn case_license_header_present() -> Result<CaseReport, TaskDriverError> {
    crate::tasks::task_158::check_attribution(&["src/wire.rs", "src/tasks/task_152.rs"])
        .map(|mut r| {
            r.case = CASES[2].to_string();
            r
        })
        .map_err(|e| arm_error("license", e))
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "unknown_fields_ignored" => case_unknown_fields_ignored(),
        "static_scan_no_deny" => case_static_scan_no_deny(),
        "license_header_present" => case_license_header_present(),
        _ => Err(arm_error(
            "case",
            format!("task-152: unknown case '{case}'"),
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
            where_: "task-152".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-152".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
