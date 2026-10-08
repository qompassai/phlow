//! task-78: model download and cache budgets (rust).
//!
//! The design asks for the model manager seam: HF hub download,
//! resume, and local cache under explicit disk/network budgets —
//! observed-byte caps independent of server-advertised sizes,
//! hash-verified resume, explicit eviction policy (LRU, pinned exempt),
//! and incomplete downloads never mistaken for complete models. The
//! design explicitly allows the honest alternative: document the seam
//! as absent with file evidence — the finding is the result.
//!
//! Seam recon (verified, not invented): the seam is ABSENT.
//! - Exact-token source scans over every `crates/*/src/**/*.rs`:
//!   `download` → 0 hits; `downloads` → 1 hit, the safe-runtime policy
//!   in `crates/phlow-runtime/src/prompt.rs` DENYING downloads ("No
//!   arbitrary commands, cwd overrides, downloads, plugins or
//!   outside-workspace access"); `huggingface` → 0 hits; `hf` → 1 hit
//!   outside gauntlet harness vocabulary: the autoresearch proposer's
//!   planner specialist model name ("hf-nemotron-..."), a name Ollama
//!   serves — classified, not a hub client.
//! - `snapshot` / `blob` / `revision` hits exist but every one is in
//!   unrelated senses: workspace file snapshots (phlow-workspace),
//!   editor snapshots (phlow-editor), run snapshots and report
//!   revisions (phlow-runtime), context snapshots (phlow-agent),
//!   experiment snapshots and manifests (phlow-experiment), council
//!   workflow revisions (phlow-council), check-runner revisions
//!   (phlow-checks) — none is a model blob cache, resume, or revision
//!   pin.
//! - Model bytes never enter phlow's address space: models are served
//!   by Ollama over HTTP (`phlow-llm`). The byte caps that DO exist
//!   guard other seams: `RESPONSE_BYTES_MAX` (2 MiB, phlow-llm
//!   transport — response bodies) and `FILE_BYTES_MAX` (256 KiB,
//!   phlow-workspace — file reads). Neither governs model downloads.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented on gauntlet
//! authority): whether phlow should gain an HF model manager
//! (download with observed-byte caps, hash-verified resume, blob cache
//! with explicit eviction, incomplete markers). Ollama owns model
//! fetching today; that is a product decision, not a bug fix.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-78";
/// Human-readable name.
pub const NAME: &str = "model download and cache budgets";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_download_client",
    "no_snapshot_revision_cache",
    "no_observed_byte_cap",
    "incomplete_never_marked",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-78 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// The source probe itself failed.
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
                write!(f, "task-78: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-78: probe {what} failed: {detail}")
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
            metrics: serde_json::json!({}),
            evidence,
            failures: vec![failure],
        }
    }
}

// ---------------------------------------------------------------------------
// Source probe (task_48 scan pattern)
// ---------------------------------------------------------------------------

/// Maximum source files the probe may read.
const SOURCE_FILES_MAX: usize = 4000;
/// Maximum bytes per source file the probe reads.
const SOURCE_BYTES_MAX: usize = 512 * 1024;

/// Workspace root: two levels above this crate's manifest directory.
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

/// This probe's own source file, excluded by exact path.
fn own_source_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("tasks")
        .join("task_78.rs")
}

/// Exact-token (case-insensitive) hits for each token over every
/// `crates/*/src/**/*.rs`, own file excluded. Returns
/// `(token, path:line)` hits. Bounded like task_48.
fn scan_sources(root: &Path, tokens: &[&str]) -> Result<Vec<(String, String)>, DriverError> {
    let own = own_source_file();
    let wanted: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();
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
                && path != own
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
                    for token in &wanted {
                        let found = line
                            .split(|c: char| !c.is_alphanumeric())
                            .any(|tok| tok.eq_ignore_ascii_case(token));
                        if found {
                            hits.push((
                                token.clone(),
                                format!("{}:{}", path.display(), lineno + 1),
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(hits)
}

/// The crate name for a hit path (`crates/<name>/...`), if present.
fn hit_crate(hit: &str) -> Option<String> {
    let mut parts = hit.split('/');
    let mut prev = String::new();
    for part in &mut parts {
        if prev == "crates" {
            return Some(part.to_string());
        }
        prev = part.to_string();
    }
    None
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: no download client exists. The only `downloads` hit in the
/// workspace is the safe-runtime policy DENYING downloads; there is
/// no HF hub client, no pull path, nothing that fetches model bytes.
fn case_no_download_client() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_download_client";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let hits = scan_sources(&root, &["download", "downloads", "huggingface", "hf"])?;
    evidence.push(format!(
        "scan tokens [download, downloads, huggingface, hf] over crates/*/src: {} hit(s)",
        hits.len()
    ));
    for (token, hit) in &hits {
        evidence.push(format!("hit: token={token} at {hit}"));
        let is_denial = hit.contains("phlow-runtime/src/prompt.rs");
        let is_harness = hit_crate(hit).is_some_and(|c| c == "phlow-gauntlet");
        // An `hf` hit in the autoresearch proposer is the planner
        // specialist's model NAME string ("hf-nemotron-...") — a name
        // Ollama serves over HTTP, not a hub client or pull path.
        // Only the `hf` token classifies this way; a `download` or
        // `huggingface` hit there would still fail the case.
        let is_model_name =
            token.as_str() == "hf" && hit.contains("phlow-autoresearch/src/ollama_proposer.rs");
        if is_model_name {
            evidence.push(format!(
                "classified (specialist model name string, not a hub client): {hit}"
            ));
        }
        if !is_denial && !is_harness && !is_model_name {
            return Ok(CaseReport::fail(
                CASE,
                format!(
                    "unclassified download-vocabulary hit at {hit}: a download seam may \
                     exist — finding refuted, inspect before proceeding"
                ),
                evidence,
            ));
        }
    }
    evidence.push(
        "every hit classified: the product hits are prompt.rs's safe-runtime \
         policy DENYING downloads (\"No arbitrary commands, cwd overrides, downloads, \
         plugins or outside-workspace access\") and the autoresearch proposer's \
         HF-prefixed specialist model name; remaining hits are gauntlet harness \
         vocabulary — no download client, no HF hub client, no pull path exists"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"hits": hits.len(), "download_clients": 0}),
        evidence,
    ))
}

/// V2: no snapshot / blob-cache / revision machinery for models.
/// Hits exist but every one is in an unrelated sense; the case
/// classifies each hit's crate and fails loudly on anything
/// unclassified.
fn case_no_snapshot_revision_cache() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_snapshot_revision_cache";
    let mut evidence = Vec::new();
    // Crates whose snapshot/revision vocabulary is verified unrelated:
    // workspace file snapshots, editor snapshots, run snapshots and
    // report revisions, context snapshots, experiment snapshots and
    // manifests, council workflow revisions, check-runner revisions,
    // approval-record snapshots (owned Record copies, not model data) —
    // plus the gauntlet harness's own probe vocabulary.
    const CLASSIFIED_CRATES: [&str; 9] = [
        "phlow-workspace",
        "phlow-editor",
        "phlow-runtime",
        "phlow-agent",
        "phlow-experiment",
        "phlow-council",
        "phlow-checks",
        "phlow-gauntlet",
        "phlow-approval",
    ];
    let root = workspace_root()?;
    let hits = scan_sources(&root, &["snapshot", "blob", "revision"])?;
    evidence.push(format!(
        "scan tokens [snapshot, blob, revision] over crates/*/src: {} hit(s)",
        hits.len()
    ));
    for (token, hit) in &hits {
        let crate_name = hit_crate(hit).unwrap_or_default();
        if !CLASSIFIED_CRATES.contains(&crate_name.as_str()) {
            return Ok(CaseReport::fail(
                CASE,
                format!(
                    "unclassified {token} hit in crate {crate_name} at {hit}: a model \
                     cache/resume/revision seam may exist — finding refuted, inspect"
                ),
                evidence,
            ));
        }
    }
    evidence.push(format!(
        "all {} hits classified into unrelated senses (workspace/editor/run/context/\
         experiment snapshots, approval-record snapshots, report/manifest/workflow \
         revisions, harness vocabulary): no blob cache, no resume logic, no model \
         revision pinning",
        hits.len()
    ));
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"hits": hits.len(), "model_caches": 0}),
        evidence,
    ))
}

/// A1: the adversarial "repo metadata advertises 2GB but serves 40GB"
/// needs an observed-byte cap on the downloader. There is no
/// downloader, so there is no cap. The byte caps that DO exist guard
/// other seams — cited as file evidence that the model-download seam
/// has no cap because it has no code.
fn case_no_observed_byte_cap() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_observed_byte_cap";
    let evidence = vec![
        "no downloader exists (V1), so no observed-byte cap can exist: a lying \
         Content-Length (2GB advertised, 40GB served) has no phlow code to bound it"
            .to_string(),
        "file evidence — the byte caps that DO exist guard other seams: \
         RESPONSE_BYTES_MAX = 2 MiB (crates/phlow-llm/src/transport.rs: response \
         bodies over the Ollama HTTP transport)"
            .to_string(),
        "file evidence — FILE_BYTES_MAX = 256 KiB \
         (crates/phlow-workspace/src/workspace.rs: file reads through the \
         containment-checked workspace)"
            .to_string(),
        "neither cap governs model downloads: model bytes never enter phlow's \
         address space — Ollama serves them over HTTP and Ollama owns the pull"
            .to_string(),
    ];
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"observed_byte_caps_on_downloads": 0}),
        evidence,
    ))
}

/// A2: "hub unreachable mid-download — the partial file is marked
/// incomplete and never mistaken for a complete model." With no
/// download path there are no partial files and no markers: phlow
/// cannot mistake an incomplete download for a model because it never
/// handles model files at all.
fn case_incomplete_never_marked() -> Result<CaseReport, DriverError> {
    const CASE: &str = "incomplete_never_marked";
    let evidence = vec![
        "no download path (V1) means no partial model files and no \
         incomplete markers: there is nothing to mark, and nothing loadable"
            .to_string(),
        "no resume logic (V2) means no hash-verified prefix check: resume \
         integrity is unrepresentable without a downloader"
            .to_string(),
        "eviction policy is unrepresentable too: no blob cache exists, so no \
         LRU, no pinned set, and no risk of evicting the active model — \
         vacuous, not enforced"
            .to_string(),
        "the design's pass criteria (observed-byte cap, hash-verified resume, \
         incomplete-never-loadable, pinned-exempt eviction) need a model \
         manager; the manager is absent — the absence, documented with the \
         scans above, IS the finding"
            .to_string(),
    ];
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"partial_file_markers": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_download_client" => case_no_download_client(),
        "no_snapshot_revision_cache" => case_no_snapshot_revision_cache(),
        "no_observed_byte_cap" => case_no_observed_byte_cap(),
        "incomplete_never_marked" => case_incomplete_never_marked(),
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
        "recon: the model-manager seam is absent — exact-token scans find no download client (sole `downloads` hit is prompt.rs's safe-runtime policy denying downloads), no huggingface client, the sole `hf` product hit is the autoresearch proposer's planner model name string (classified), and every snapshot/blob/revision hit is an unrelated sense (workspace/editor/run/context/experiment snapshots, report/manifest/workflow revisions)".to_string(),
        "recon: model bytes never enter phlow's address space — models are served by Ollama over HTTP (phlow-llm); the existing byte caps guard other seams (RESPONSE_BYTES_MAX = 2 MiB on Ollama response bodies; FILE_BYTES_MAX = 256 KiB on workspace file reads)".to_string(),
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
        "finding: none of the design's pass criteria can be met — no observed-byte cap (no downloader), no hash-verified resume (no resume logic), incomplete downloads are trivially never loadable (no model files), eviction policy is vacuous (no cache)".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent, documented with file evidence: no model download/cache manager exists in any phlow crate — exact-token scans over crates/*/src/**/*.rs find `download` 0 hits, `downloads` 1 hit (crates/phlow-runtime/src/prompt.rs: the safe-runtime policy denying downloads), `huggingface` 0 hits, `hf` 1 product hit (the autoresearch proposer's HF-prefixed planner model name, classified — a served name, not a client), and every `snapshot`/`blob`/`revision` hit classified into unrelated senses (workspace/editor/run/context/experiment snapshots; report/manifest/workflow revisions). Model bytes never enter phlow's address space: Ollama serves models over HTTP and owns the pull. The design's pass criteria (observed-byte cap independent of advertised sizes, hash-verified resume, incomplete-never-loadable, pinned-exempt eviction) need a downloader, and there is none. Whether phlow should gain an HF model manager (download with observed-byte caps, hash-verified resume, blob cache with explicit eviction, incomplete markers) is a product decision for Matt, not a gauntlet-authorized change.".to_string(),
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
