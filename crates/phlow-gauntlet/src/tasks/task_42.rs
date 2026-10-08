//! task-42: audit log append-only (rust).
//!
//! Recon probe: the design asks for a tamper-evident journal seam — an
//! event writer whose entries are verifiable after the fact (each entry
//! linked to the previous one, a sealing key, a runnable verification
//! check that names the tampered entry). The adversarial scenarios are
//! a flipped byte in an old entry (verification fails with the entry
//! index), a truncated tail (length plus head-hash mismatch), and a
//! whole-log rewrite (requires the sealing key — fails without it).
//!
//! Honest result: the seam is ABSENT from the runtime/approval path.
//! No runtime phlow crate has an event writer with post-hoc
//! verifiability. The runtime evidence is threefold, all gathered at
//! probe time from the working tree:
//!
//! 1. Mechanism-vocabulary scan: a walk over every
//!    `crates/*/src/**/*.rs` finds zero integrity-mechanism tokens in
//!    the runtime/approval path — no entry-linking, no sealing keys,
//!    no tamper-evidence journaling. The scan DOES hit in exactly one
//!    place: `phlow-autoresearch`, whose hash-chained experiment
//!    ledger is the sanctioned experimental exception (Matt's ruling,
//!    below) — classified, counted separately, never the runtime seam.
//!    (The probe's own task file is excluded from hits by exact path:
//!    the task NAME itself contains the design vocabulary. The probe
//!    prose is additionally written to avoid the literal tokens, so
//!    the exclusion changes nothing for the other files.)
//! 2. Grow-only structures that DO exist are classified and rejected
//!    as the seam: `ConsumedApprovals` in phlow-experiment persists
//!    single-use approval ids as one-JSON-object-per-line JSONL for
//!    restart safety — replay protection, not an event journal; it has
//!    no entry linking, no sealing key, no verifier, and entries
//!    EXPIRE (`evict_expired` drops dead entries, the opposite of a
//!    journal). `FusionReceipt.log` in phlow-tools is an in-memory
//!    `Vec<String>` decision log — not persisted, no integrity at all.
//!    `ApprovalQueue.records` in phlow-approval is a bounded in-memory
//!    `Vec<Record>` decision buffer — "append-only" only in the sense
//!    that a full queue rejects new requests instead of evicting old
//!    decisions; not persisted, no entry linking, no sealing key, no
//!    verifier.
//! 3. The design's adversarial weapons have no target in the
//!    runtime/approval path: there is no log file format to flip a
//!    byte in, no verifier to name an entry index, no head-hash or
//!    length tracking to catch a truncation.
//!
//! Four cases, all against the real working tree (no mocks): two
//! validation, two adversarial. The task-level verdict is `fail` at
//! `"seam"` because the design's pass criteria (post-hoc modification
//! *detectable* with the tampered entry identified; verification a
//! runnable check) need a journal to attach to in the runtime/approval
//! path, and none exists there.
//!
//! Ruled by Matt (2026-10-08): no journal seam in the runtime/approval
//! path; the autoresearch ledger is the sanctioned experimental
//! exception. The question this probe originally banked — whether
//! phlow should gain a sealed, entry-linked journal — is answered for
//! the experiment surface (yes: the autoresearch ledger, hash-chained,
//! experimental crate only) and remains open for the runtime: the
//! approval/promotion path (which already persists consumed approval
//! ids) would be the natural first consumer if it is ever adopted
//! there. This probe fails closed if integrity vocabulary appears
//! anywhere outside the sanctioned exception.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-42";
/// Human-readable name.
pub const NAME: &str = "audit log append-only";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_integrity_tokens_in_sources",
    "grow_only_stores_lack_verification",
    "byte_flip_has_no_verifier",
    "truncation_has_no_head_hash",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-42 driver itself (not of the code under test).
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
                write!(f, "task-42: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-42: cannot probe {what}: {detail}")
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

/// Integrity-mechanism tokens, assembled at runtime from halves so the
/// probe's own source never contains the literal tokens it scans for.
/// These name the design's required machinery: entry linking between
/// consecutive records, a sealing key, tamper-evidence journaling.
fn mechanism_tokens() -> Vec<String> {
    const HALVES: [(&str, &str); 10] = [
        ("audit", "_log"),
        ("audit ", "log"),
        ("audit", "_event"),
        ("audit ", "event"),
        ("hash", "_chain"),
        ("hash", " chain"),
        ("tamper", "-evident"),
        ("tamper", "_evident"),
        ("sealing", " key"),
        ("sealing", "_key"),
    ];
    HALVES.iter().map(|(a, b)| format!("{a}{b}")).collect()
}

/// Grow-only vocabulary tokens, assembled at runtime. Hits are
/// EXPECTED (the replay store, the fusion decision log, this task's
/// own NAME) and are classified in V2 rather than treated as the seam.
fn grow_tokens() -> Vec<String> {
    const HALVES: [(&str, &str); 2] = [("append", "-only"), ("append", "_only")];
    HALVES.iter().map(|(a, b)| format!("{a}{b}")).collect()
}

/// Partition scan hits into (sanctioned experimental exception,
/// runtime/approval path). The exception is exactly one crate:
/// `phlow-autoresearch`, whose hash-chained experiment ledger Matt
/// sanctioned as the experimental exception (2026-10-08). Everything
/// else is the runtime/approval path, where the seam stays absent.
fn partition_hits(hits: &[String]) -> (Vec<String>, Vec<String>) {
    hits.iter()
        .cloned()
        .partition(|h| h.contains("crates/phlow-autoresearch/"))
}

/// Walk `crates/` under the workspace root and return every
/// `path: token` hit for `.rs` files inside a `src` tree, skipping the
/// whole phlow-gauntlet probe harness (it is the scanner, not the
/// product — its prose discusses other tasks' audit vocabulary).
/// Bounded: files over [`SOURCE_BYTES_MAX`]
/// are skipped, and the walk stops after [`SOURCE_FILES_MAX`] files.
fn scan_sources(root: &Path, tokens: &[String]) -> Result<Vec<String>, DriverError> {
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
            if path.components().any(|c| c.as_os_str() == "phlow-gauntlet") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.components().any(|c| c.as_os_str() == "src")
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
                for token in tokens {
                    if text.contains(token.as_str()) {
                        hits.push(format!("{}: {token}", path.display()));
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

/// V1: no integrity-mechanism vocabulary in the runtime/approval
/// path. The scan covers every `crates/*/src/**/*.rs` except this
/// probe's own harness — the tokens are assembled at runtime so the
/// probe cannot match its own prose. Hits inside the sanctioned
/// experimental exception (`phlow-autoresearch`, the hash-chained
/// experiment ledger) are classified and counted separately; a hit
/// anywhere else fails the case.
fn case_no_integrity_tokens_in_sources() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_integrity_tokens_in_sources";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let tokens = mechanism_tokens();
    let hits = scan_sources(&root, &tokens)?;
    let (exception_hits, runtime_hits) = partition_hits(&hits);
    evidence.push(format!(
        "scanned {} integrity-mechanism tokens over crates/*/src (phlow-gauntlet harness excluded); runtime/approval-path hits: {}",
        tokens.len(),
        runtime_hits.len()
    ));
    evidence.push(format!(
        "sanctioned experimental exception (phlow-autoresearch ledger, Matt's 2026-10-08 ruling): {} hits",
        exception_hits.len()
    ));
    for hit in &exception_hits {
        evidence.push(format!("exception hit: {hit}"));
    }
    if !runtime_hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "integrity-mechanism vocabulary found in the runtime/approval path: {}",
                runtime_hits.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "zero runtime/approval-path hits: outside the sanctioned experimental exception, no phlow source names entry linking, sealing keys, or tamper-evidence journaling"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"integrity_tokens": tokens.len(), "integrity_hits": 0, "sanctioned_exception_hits": exception_hits.len()}),
        evidence,
    ))
}

/// V2: the grow-only structures that DO exist lack any verification
/// machinery, so they are not the design's seam. `ConsumedApprovals`
/// (phlow-experiment/src/promotion.rs) persists single-use approval
/// ids as one-JSON-object-per-line JSONL for restart safety — replay
/// protection, not an event journal: entries EXPIRE
/// (`evict_expired`), the opposite of a journal, and there is no entry
/// linking, no sealing key, no verifier. `FusionReceipt.log`
/// (phlow-tools/src/solpi/action_fusion.rs) is an in-memory
/// `Vec<String>` decision log — not persisted, no integrity at all.
/// `ApprovalQueue.records` (phlow-approval/src/queue.rs) is a bounded
/// in-memory `Vec<Record>` — "append-only" only as a doc note that a
/// full queue rejects new requests rather than evicting old decisions;
/// no persistence, no linking, no sealing key, no verifier.
fn case_grow_only_stores_lack_verification() -> Result<CaseReport, DriverError> {
    const CASE: &str = "grow_only_stores_lack_verification";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let tokens = grow_tokens();
    let hits = scan_sources(&root, &tokens)?;
    evidence.push(format!(
        "grow-only vocabulary hits (phlow-gauntlet harness excluded): {}",
        hits.len()
    ));
    for hit in &hits {
        evidence.push(format!("hit: {hit}"));
    }
    // Every hit must be one of the three known non-seam structures,
    // this task's own NAME echoed in gauntlet docs, or the sanctioned
    // experimental exception (the autoresearch ledger's own doc
    // vocabulary — a real journal, classified as the exception, not
    // one of the runtime grow-only stores); anything else is a
    // finding to surface.
    let mut unexplained = Vec::new();
    for hit in &hits {
        if hit.contains("promotion.rs")
            || hit.contains("action_fusion.rs")
            || hit.contains("queue.rs")
            || hit.contains("task_36.rs")
        {
            continue;
        }
        if hit.contains("crates/phlow-autoresearch/") {
            evidence.push(format!(
                "classified (sanctioned experimental exception — the autoresearch ledger, not a runtime grow-only store): {hit}"
            ));
            continue;
        }
        unexplained.push(hit.clone());
    }
    if !unexplained.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "unexplained grow-only vocabulary hits: {}",
                unexplained.join("; ")
            ),
            evidence,
        ));
    }
    let promotion = std::fs::read_to_string(
        root.join("crates")
            .join("phlow-experiment")
            .join("src")
            .join("promotion.rs"),
    )
    .map_err(|e| fixture_error("promotion.rs", e))?;
    if promotion.contains("evict_expired") {
        evidence.push(
            "promotion.rs: the JSONL store evicts expired entries (evict_expired) — entries are NOT retained; not a journal"
                .to_string(),
        );
    }
    let mech = mechanism_tokens();
    let mech_hits = scan_sources(&root, &mech)?;
    let near_stores: Vec<&String> = mech_hits
        .iter()
        .filter(|h| h.contains("promotion.rs") || h.contains("action_fusion.rs"))
        .collect();
    evidence.push(format!(
        "integrity-mechanism tokens inside the two grow-only structures: {}",
        near_stores.len()
    ));
    if !near_stores.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "verification machinery found near the grow-only stores: {}",
                near_stores
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "both grow-only structures lack entry linking, sealing keys, and a verifier — adjacent, not the seam"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"grow_only_hits": hits.len(), "integrity_hits_near_stores": 0}),
        evidence,
    ))
}

/// A1: the design's adversarial weapon — flip a byte in an old entry,
/// expect verification to fail naming the entry index — has no
/// target in the runtime/approval path. With no event writer (V1)
/// and no verifier (V2) there, there is no log file format to tamper
/// and no verification entry point to run. (The sanctioned
/// experimental ledger in phlow-autoresearch HAS a verifier — for its
/// own experiment records; it guards no runtime/approval record and
/// is never linked into the promotion path.) The case passes as a
/// probe: it documents the weapon and the missing target, rather than
/// claiming a detection that was never measured.
fn case_byte_flip_has_no_verifier() -> Result<CaseReport, DriverError> {
    const CASE: &str = "byte_flip_has_no_verifier";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let tokens = mechanism_tokens();
    let hits = scan_sources(&root, &tokens)?;
    let (exception_hits, runtime_hits) = partition_hits(&hits);
    evidence.push(format!(
        "integrity hits: {} in the runtime/approval path, {} in the sanctioned experimental exception",
        runtime_hits.len(),
        exception_hits.len()
    ));
    if !runtime_hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "integrity machinery exists in the runtime/approval path ({}), so a byte-flip has a target after all: {}",
                runtime_hits.len(),
                runtime_hits.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "adversarial input (flip a byte in an old entry) would need a writer and a verifier in the runtime/approval path; the V1/V2 scans show neither exists there"
            .to_string(),
    );
    evidence.push(
        "no tamper detection was measured in the runtime/approval path because there is no journal to tamper there — the workspace's one journal is the sanctioned experimental ledger, which guards no runtime record; a claimed detection against the runtime path would be invented, not sourced"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"verifiers_found": 0}),
        evidence,
    ))
}

/// A2: truncation and whole-log rewrite have no detector. No
/// head-hash or length tracking exists anywhere (V1 scan), and the
/// closest store — the replay JSONL — drops expired entries by
/// design, so even "the tail is shorter than yesterday" is normal
/// operation there, not evidence of tampering. The design's
/// "truncation detected via length plus head-hash mismatch" and
/// "rewrite requires the sealing key" have nothing to attach to.
fn case_truncation_has_no_head_hash() -> Result<CaseReport, DriverError> {
    const CASE: &str = "truncation_has_no_head_hash";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let tokens = mechanism_tokens();
    let hits: Vec<String> = scan_sources(&root, &tokens)?
        .into_iter()
        .filter(|h| h.contains("promotion.rs"))
        .collect();
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "integrity vocabulary near the replay store: {}",
                hits.join("; ")
            ),
            evidence,
        ));
    }
    evidence.push(
        "no head-hash or length tracking exists in the runtime/approval path: the V1 scan found zero integrity-mechanism tokens there (the sanctioned exception's head hash lives in the experimental crate and tracks no runtime record)"
            .to_string(),
    );
    evidence.push(
        "the replay store evicts expired entries by design, so a shorter tail is normal operation there — truncation is undetectable AND unremarkable"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"head_hash_trackers": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_integrity_tokens_in_sources" => case_no_integrity_tokens_in_sources(),
        "grow_only_stores_lack_verification" => case_grow_only_stores_lack_verification(),
        "byte_flip_has_no_verifier" => case_byte_flip_has_no_verifier(),
        "truncation_has_no_head_hash" => case_truncation_has_no_head_hash(),
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
        "recon: integrity-mechanism vocabulary scan over every crates/*/src — zero hits in the runtime/approval path (entry linking, sealing keys, tamper-evidence journaling all absent there); the only hits are the sanctioned experimental exception (the phlow-autoresearch ledger, Matt's 2026-10-08 ruling)".to_string(),
        "recon: the three grow-only structures that exist in the runtime are not the seam — ConsumedApprovals (phlow-experiment/src/promotion.rs) is replay protection with expiring entries; FusionReceipt.log (phlow-tools/src/solpi/action_fusion.rs) is an in-memory Vec<String>; ApprovalQueue.records (phlow-approval/src/queue.rs) is a bounded in-memory Vec<Record>".to_string(),
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
        "finding: no tamper-evident journal exists in phlow's runtime/approval path — the design's adversarial weapons (byte flip, tail truncation, whole-log rewrite) have no target there; the workspace's one journal is the sanctioned experimental exception".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no tamper-evident journal exists in the runtime/approval path of any phlow crate — a runtime vocabulary scan finds zero integrity-mechanism tokens there (no entry linking, no sealing keys, no tamper-evidence journaling), the grow-only structures that exist in the runtime are not the seam (the approval replay store persists single-use ids as JSONL with expiring entries and no verifier; the fusion decision log is an in-memory Vec<String>), and the workspace's one real journal — the hash-chained autoresearch experiment ledger — is the sanctioned experimental exception (Matt, 2026-10-08), never linked into the promotion path. The design's pass criteria (post-hoc modification detectable with the tampered entry identified; verification a runnable check) need a journal to attach to in the runtime/approval path, and there is none.".to_string(),
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
