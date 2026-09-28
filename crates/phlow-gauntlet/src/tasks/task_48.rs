//! task-48: disk quota enforcement (rust).
//!
//! Recon probe: the design asks for a per-run disk-quota seam — artifact
//! writers that refuse with a typed `QuotaExceeded` when a run's bytes
//! pass its quota, that leave no partial artifact behind, and that fail
//! fast when the quota is zero. The adversarial scenarios are an
//! artifact stream that exceeds the quota mid-write (the partial file is
//! removed or marked incomplete) and a zero quota (the run fails at the
//! first write, not after doing work).
//!
//! Honest result: the seam is ABSENT. The evidence is gathered at probe
//! time from the live working tree:
//!
//! 1. Vocabulary scan: a tokenized walk over every `crates/*/src/**/*.rs`
//!    finds zero standalone `quota` tokens and zero `QuotaExceeded` /
//!    `quota_exceeded` tokens. (Substring matching would false-positive on
//!    "quotations" in the reducer docs, so the probe tokenizes on
//!    non-alphanumerics and compares case-insensitively; the probe's own
//!    file is excluded by exact path because the task NAME contains the
//!    design vocabulary.)
//! 2. The closest byte bounds that DO exist are not write quotas: the
//!    workspace `FILE_BYTES_MAX` is a per-read cap (constant, compile
//!    time, read path only), and the experiment scheduler's budgets are
//!    step budgets, not disk budgets. No writer tracks per-run cumulative
//!    bytes against a limit — a token scan for byte-accounting vocabulary
//!    (`bytes_written`, `written_bytes`, `disk_usage`, `byte_budget`)
//!    returns zero.
//! 3. The design's failure modes have no error type and no writer to
//!    attach to: no `QuotaExceeded` exists anywhere, and there is no
//!    per-run artifact/transcript writer (task-33 already found the
//!    checkpoint API is memory-only; task-35 found events are an
//!    in-memory `Vec<String>`).
//!
//! Four cases, all against the real working tree (no mocks): two
//! validation, two adversarial. The task-level verdict is `fail` at
//! `"seam"` because the design's pass criteria (per-run usage never
//! exceeds quota + one block, measured; no partial artifact mistaken for
//! complete) need a quota-enforcing writer, and none exists.
//!
//! Banked for Matt (product decision, NOT auto-implemented): whether
//! run artifact writers should gain per-run byte quotas with a typed
//! `QuotaExceeded` and partial-file hygiene (remove or mark incomplete).

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-48";
/// Human-readable name.
pub const NAME: &str = "disk quota enforcement";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_quota_tokens_in_sources",
    "write_paths_have_no_byte_accounting",
    "mid_write_quota_exceeded_has_no_target",
    "zero_quota_fast_fail_has_no_target",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-48 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture (workspace root, source tree) was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// A probe step failed.
    Probe {
        /// What was being probed.
        what: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-48: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-48: cannot probe {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

fn fixture_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Fixture {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

fn probe_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Probe {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Fixtures: the working tree is the task source
// ---------------------------------------------------------------------------

/// Workspace root: two levels above this crate's manifest directory.
/// The probe reads the live working tree, never a cached copy.
fn workspace_root() -> Result<PathBuf, DriverError> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| fixture_error("workspace root", "manifest dir has no grandparent"))?;
    if !root.join("Cargo.lock").is_file() {
        return Err(fixture_error(
            "workspace root",
            format!("no Cargo.lock under {}", root.display()),
        ));
    }
    Ok(root.to_path_buf())
}

/// The gauntlet crate's own `src` tree, excluded from scan hits by path
/// prefix: the harness's probes legitimately use design vocabulary —
/// task names (`disk quota enforcement`), absence discussions, and
/// scan tokens themselves — so any probe file would otherwise
/// self-match. Only product crates count toward the finding.
/// (Wave 81-85: task-82's rate-limit probe discusses per-provider
/// quota absence and broke the original own-file-only exclusion.)
fn harness_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Walk `crates/` under the workspace root and return `path:line` hits
/// for `.rs` files inside a `src` tree whose alphanumeric-token stream
/// contains `token` (case-insensitive, exact token — not a substring, so
/// "quotations" never matches "quota"). Skips the gauntlet harness's
/// own `src` tree by path prefix (see [`harness_src_dir`]). Bounded:
/// files over [`SOURCE_BYTES_MAX`] are skipped, and the walk stops
/// after [`SOURCE_FILES_MAX`] files.
fn scan_sources(root: &Path, token: &str) -> Result<Vec<String>, DriverError> {
    let excluded = harness_src_dir();
    let wanted = token.to_lowercase();
    let crates_dir = root.join("crates");
    let mut hits = Vec::new();
    let mut files_seen = 0usize;
    let mut stack = vec![crates_dir];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| fixture_error("source walk", format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| fixture_error("source walk", e))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.components().any(|c| c.as_os_str() == "src")
                && !path.starts_with(&excluded)
            {
                files_seen += 1;
                if files_seen > SOURCE_FILES_MAX {
                    return Err(probe_error(
                        "source scan",
                        format!("file budget {SOURCE_FILES_MAX} exhausted"),
                    ));
                }
                let bytes = std::fs::read(&path).map_err(|e| {
                    fixture_error("source read", format!("{}: {e}", path.display()))
                })?;
                if bytes.len() > SOURCE_BYTES_MAX {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                for (lineno, line) in text.lines().enumerate() {
                    let found = line
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|tok| tok.eq_ignore_ascii_case(&wanted));
                    if found {
                        hits.push(format!("{}:{}", path.display(), lineno + 1));
                    }
                }
            }
        }
    }
    Ok(hits)
}

// ---------------------------------------------------------------------------
// Case verdicts
// ---------------------------------------------------------------------------

/// The parsed verdict of one case.
#[derive(Debug, Clone)]
pub struct CaseReport {
    /// Which case ran.
    pub case: String,
    /// Whether the case's own assertions held.
    pub passed: bool,
    /// Measured numbers.
    pub metrics: serde_json::Value,
    /// Diagnostic lines from the case.
    pub evidence: Vec<String>,
    /// Failing assertion details, empty when `passed`.
    pub failures: Vec<String>,
}

impl CaseReport {
    fn pass(case: &'static str, metrics: serde_json::Value, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: true,
            metrics,
            evidence,
            failures: Vec::new(),
        }
    }

    fn fail(case: &'static str, failure: String, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: false,
            metrics: serde_json::Value::Null,
            evidence,
            failures: vec![failure],
        }
    }
}

/// V1: no standalone `quota` token anywhere in the workspace sources.
/// Tokenized (not substring) matching, so the reducer's "quotations"
/// vocabulary cannot false-positive; the gauntlet harness's own `src`
/// tree is prefix-excluded because probe files legitimately use the
/// design vocabulary (task names, absence discussions, scan tokens).
fn case_no_quota_tokens_in_sources() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_quota_tokens_in_sources";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let hits = scan_sources(&root, "quota")?;
    evidence.push(format!(
        "tokenized scan for the exact token 'quota' over crates/*/src (gauntlet harness src prefix-excluded); hits: {}",
        hits.len()
    ));
    for hit in &hits {
        evidence.push(format!("hit: {hit}"));
    }
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("quota vocabulary found: {}", hits.join("; ")),
            evidence,
        ));
    }
    evidence.push(
        "zero hits: no phlow source names a disk quota, a quota limit, or quota enforcement"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"quota_tokens": 1, "quota_hits": 0}),
        evidence,
    ))
}

/// V2: the byte bounds that DO exist are not write quotas. The
/// workspace `FILE_BYTES_MAX` (256 KiB) is a per-read cap — a
/// compile-time constant on the read path (`file.take(FILE_BYTES_MAX +
/// 1)`), not a per-run write limit — and the experiment scheduler's
/// budgets are step budgets, not disk budgets. No writer tracks
/// per-run cumulative bytes against a limit: the byte-accounting
/// vocabulary scan returns zero.
fn case_write_paths_have_no_byte_accounting() -> Result<CaseReport, DriverError> {
    const CASE: &str = "write_paths_have_no_byte_accounting";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let mut accounting_hits = Vec::new();
    for token in [
        "bytes_written",
        "written_bytes",
        "disk_usage",
        "byte_budget",
    ] {
        let hits = scan_sources(&root, token)?;
        evidence.push(format!("token '{token}': {} hits", hits.len()));
        accounting_hits.extend(hits);
    }
    if !accounting_hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "byte-accounting vocabulary found: {}",
                accounting_hits.join("; ")
            ),
            evidence,
        ));
    }
    // Confirm the closest bound is the read cap: FILE_BYTES_MAX is a
    // `pub const` (compile-time, not settable per run) and its use site
    // is the read path.
    let workspace_rs = std::fs::read_to_string(
        root.join("crates")
            .join("phlow-workspace")
            .join("src")
            .join("workspace.rs"),
    )
    .map_err(|e| fixture_error("workspace.rs", e))?;
    let is_const = workspace_rs.contains("pub const FILE_BYTES_MAX");
    let on_read_path = workspace_rs.contains("file.take(FILE_BYTES_MAX + 1)");
    evidence.push(format!(
        "FILE_BYTES_MAX: pub const={is_const}, applied on the read path={on_read_path}"
    ));
    if !is_const || !on_read_path {
        return Ok(CaseReport::fail(
            CASE,
            "FILE_BYTES_MAX is not the expected compile-time read cap — re-audit needed"
                .to_string(),
            evidence,
        ));
    }
    evidence.push(
        "no writer tracks per-run cumulative bytes against a limit — the read cap is the closest byte bound and it is neither per-run nor on the write path"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"accounting_tokens": 4, "accounting_hits": 0}),
        evidence,
    ))
}

/// A1: the mid-write scenario's failure mode has no error type and no
/// writer to attach to. `QuotaExceeded` / `quota_exceeded` appear
/// nowhere; there is no per-run artifact or transcript writer (the
/// checkpoint API is memory-only per task-33; events are an in-memory
/// `Vec<String>` per task-35), so "remove the partial file or mark it
/// incomplete" has no file to act on.
fn case_mid_write_quota_exceeded_has_no_target() -> Result<CaseReport, DriverError> {
    const CASE: &str = "mid_write_quota_exceeded_has_no_target";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let mut error_hits = Vec::new();
    for token in ["QuotaExceeded", "quota_exceeded"] {
        let hits = scan_sources(&root, token)?;
        evidence.push(format!("token '{token}': {} hits", hits.len()));
        error_hits.extend(hits);
    }
    if !error_hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "quota-exceeded error type exists after all: {}",
                error_hits.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "no QuotaExceeded error type exists anywhere — a mid-write quota breach has no typed failure to produce"
            .to_string(),
    );
    evidence.push(
        "no per-run artifact/transcript writer exists to breach mid-write (checkpoint API is memory-only; event log is an in-memory Vec<String>) — partial-file hygiene has no file to act on"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"quota_exceeded_types": 0}),
        evidence,
    ))
}

/// A2: the zero-quota scenario has no knob to set. The closest byte
/// bound (`FILE_BYTES_MAX`) is a compile-time constant, not a
/// settable per-run quota, and it guards reads — so "fail fast at the
/// first write" cannot even be configured, let alone observed.
fn case_zero_quota_fast_fail_has_no_target() -> Result<CaseReport, DriverError> {
    const CASE: &str = "zero_quota_fast_fail_has_no_target";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let hits = scan_sources(&root, "quota")?;
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!("quota vocabulary exists after all: {}", hits.join("; ")),
            evidence,
        ));
    }
    evidence.push(
        "no quota vocabulary anywhere (V1 scan repeated): there is no per-run quota to set to zero"
            .to_string(),
    );
    evidence.push(
        "the closest byte bound is FILE_BYTES_MAX — a pub const on the read path, not a settable per-run write quota — so the zero-quota fast-fail has no target"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"settable_quotas": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_quota_tokens_in_sources" => case_no_quota_tokens_in_sources(),
        "write_paths_have_no_byte_accounting" => case_write_paths_have_no_byte_accounting(),
        "mid_write_quota_exceeded_has_no_target" => case_mid_write_quota_exceeded_has_no_target(),
        "zero_quota_fast_fail_has_no_target" => case_zero_quota_fast_fail_has_no_target(),
        _ => Err(DriverError::Fixture {
            what: "case".to_string(),
            detail: format!("unknown case '{case}'"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Task entry point
// ---------------------------------------------------------------------------

struct TaskFailure {
    where_: String,
    how: String,
    evidence: Vec<String>,
}

fn run_inner(_ctx: &Ctx) -> Result<Vec<String>, TaskFailure> {
    let mut evidence = vec![
        "recon: tokenized vocabulary scan over every crates/*/src — zero standalone 'quota' tokens, zero QuotaExceeded/quota_exceeded types, zero byte-accounting tokens (bytes_written, written_bytes, disk_usage, byte_budget)".to_string(),
        "recon: the closest byte bounds are not write quotas — workspace FILE_BYTES_MAX is a compile-time per-read cap; the experiment scheduler's budgets are step budgets, not disk budgets".to_string(),
    ];
    for case in CASES {
        let report = run_case(case).map_err(|e| TaskFailure {
            where_: case.to_string(),
            how: e.to_string(),
            evidence: evidence.clone(),
        })?;
        evidence.push(format!("case {case}: passed={}", report.passed));
        evidence.push(format!("case {case} metrics: {}", report.metrics));
        for line in &report.evidence {
            evidence.push(format!("case {case}: {line}"));
        }
        if !report.passed {
            return Err(TaskFailure {
                where_: case.to_string(),
                how: report.failures.join("; "),
                evidence,
            });
        }
    }
    evidence.push(
        "finding: no per-run disk-quota seam exists — the design's failure modes (typed QuotaExceeded, partial-file hygiene, zero-quota fast-fail) have no error type, no writer, and no knob to attach to".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no per-run disk quota exists in any phlow crate — a tokenized workspace scan finds zero standalone 'quota' tokens, zero QuotaExceeded/quota_exceeded error types, and zero byte-accounting tokens (bytes_written, written_bytes, disk_usage, byte_budget); the closest byte bounds are the workspace FILE_BYTES_MAX compile-time per-read cap and the experiment scheduler's step budgets, neither a per-run write limit. The design's pass criteria (per-run usage never exceeds quota + one block, measured; no partial artifact mistaken for complete) need a quota-enforcing writer, and there is none.".to_string(),
        evidence,
    })
}

/// Attempt the task.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    match run_inner(ctx) {
        Ok(evidence) => TaskOutcome::Pass {
            evidence: bound_evidence(evidence),
        },
        Err(failure) => TaskOutcome::Fail {
            where_: failure.where_,
            how: failure.how,
            evidence: bound_evidence(failure.evidence),
        },
    }
}
