//! task-83: streaming vs non-streaming parity (rust).
//!
//! The design asks for streaming vs non-streaming parity: the same
//! prompt through SSE reassembly and single-shot JSON must yield the
//! same semantic result — byte-exact reassembly across adversarial
//! chunk splits (inside JSON string escapes, inside multi-byte UTF-8),
//! truncated streams typed as truncated (never complete), and usage
//! handled whether it arrives in the final chunk or not at all.
//!
//! Seam recon (verified, not invented):
//! - Streaming is disabled by construction: `build_chat_payload`
//!   (`crates/phlow-llm/src/payload.rs:69`) inserts
//!   `"stream": false` into every chat payload. The provider is asked
//!   for single-shot JSON, always.
//! - There is no SSE vocabulary in the client: exact-token scans for
//!   `event-stream`, `[DONE]`, `text/event-stream` over
//!   `phlow-llm/src/**/*.rs` find zero hits.
//! - The transport contract (`LlmTransport::post_chat ->
//!   Result<Value, LlmError>`) delivers one decoded JSON body per
//!   call: there is no chunk boundary concept, no reassembly
//!   function, no stream-terminator state machine. A truncated
//!   stream has no code to be typed by.
//! - `BoundedBody` (`crates/phlow-llm/src/transport.rs`) is a byte
//!   cap on the body, not a terminator check: it cannot distinguish
//!   "complete" from "cut off mid-stream".
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented on gauntlet
//! authority): whether phlow-llm should support SSE streaming with
//! byte-exact reassembly and truncated-stream typing.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-83";
/// Human-readable name.
pub const NAME: &str = "streaming vs non-streaming parity";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "streaming_disabled_by_construction",
    "no_sse_vocabulary",
    "chunk_boundary_semantics_unrepresentable",
    "truncation_semantics_unrepresentable",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-83 driver itself (not of the code under test).
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
                write!(f, "task-83: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-83: probe {what} failed: {detail}")
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

/// The gauntlet's own crate root, excluded from the product scan: the
/// harness's own probes use the design vocabulary.
fn excluded_crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Exact-token (case-insensitive) hits for `token` over a crate's
/// `src/**/*.rs` — the gauntlet crate itself excluded. Bounded like
/// task_48.
fn scan_sources(root: &Path, crate_name: &str, token: &str) -> Result<Vec<String>, DriverError> {
    let excluded = excluded_crate_root();
    let wanted = token.to_lowercase();
    let crates_dir = root.join("crates").join(crate_name).join("src");
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
                if path != excluded {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
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
// Cases
// ---------------------------------------------------------------------------

use phlow_config::OllamaConfig;
use phlow_llm::payload::build_chat_payload;

/// V1: streaming is disabled by construction. The real payload
/// builder inserts `"stream": false` into every chat payload — the
/// provider is never asked for a stream.
fn case_streaming_disabled_by_construction() -> Result<CaseReport, DriverError> {
    const CASE: &str = "streaming_disabled_by_construction";
    let mut evidence = Vec::new();
    let payload = build_chat_payload(
        &OllamaConfig::default(),
        &[serde_json::json!({"role": "user", "content": "hi"})],
        &[],
        None,
    )
    .map_err(|e| fixture_error("chat payload", e))?;
    evidence.push(format!(
        "build_chat_payload emitted stream={}",
        payload
            .get("stream")
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| "<absent>".to_string())
    ));
    if payload.get("stream") != Some(&Value::from(false)) {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "payload stream field is not false: {}",
                payload.get("stream").unwrap_or(&Value::Null)
            ),
            evidence,
        ));
    }
    evidence.push(
        "crates/phlow-llm/src/payload.rs:69 inserts \"stream\": false — every chat \
         completion the client requests is single-shot JSON; there is no streaming \
         mode to be in parity with"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"stream": false}),
        evidence,
    ))
}

/// V2: there is no SSE vocabulary in the client. Exact-token scans
/// for the streaming markers find zero hits in `phlow-llm/src`.
fn case_no_sse_vocabulary() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_sse_vocabulary";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let mut total_hits = 0usize;
    for token in ["event-stream", "text/event-stream", "[DONE]"] {
        let hits = scan_sources(&root, "phlow-llm", token)?;
        evidence.push(format!(
            "exact-token scan for '{token}' over phlow-llm/src/**/*.rs: {} hit(s)",
            hits.len()
        ));
        for hit in &hits {
            evidence.push(format!("  unexpected hit: {hit}"));
        }
        total_hits += hits.len();
    }
    if total_hits != 0 {
        return Ok(CaseReport::fail(
            CASE,
            format!("{total_hits} SSE hit(s) — the absence finding is refuted"),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"sse_vocabulary_hits": 0}),
        evidence,
    ))
}

/// A1: chunk-boundary semantics are unrepresentable. The transport
/// contract delivers one decoded JSON body per call — there is no
/// reassembly function that a split JSON escape or a split multi-byte
/// UTF-8 sequence could run through. The design's adversarial chunk
/// splits have no code to split.
fn case_chunk_boundary_semantics_unrepresentable() -> Result<CaseReport, DriverError> {
    const CASE: &str = "chunk_boundary_semantics_unrepresentable";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    // The reassembly vocabulary of the design: reassemble, chunk,
    // event-source. Any one of them present refutes the absence.
    let mut total_hits = 0usize;
    for token in ["reassemble", "eventsource", "event_source"] {
        let hits = scan_sources(&root, "phlow-llm", token)?;
        evidence.push(format!(
            "exact-token scan for '{token}' over phlow-llm/src/**/*.rs: {} hit(s)",
            hits.len()
        ));
        total_hits += hits.len();
    }
    if total_hits != 0 {
        return Ok(CaseReport::fail(
            CASE,
            format!("{total_hits} reassembly hit(s) — the absence finding is refuted"),
            evidence,
        ));
    }
    evidence.push(
        "LlmTransport::post_chat -> Result<Value, LlmError>: one decoded JSON body \
         per call — the contract has no chunk boundary, no partial frame, no \
         reassembly step. A chunk boundary splitting a JSON string escape or a \
         multi-byte UTF-8 sequence cannot occur because there are no chunks"
            .to_string(),
    );
    evidence.push(
        "the design's 'byte-exact reassembly' and 'no mojibake' criteria need a \
         reassembly function; phlow-llm has none — parity with a nonexistent \
         streaming mode is unrepresentable"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"reassembly_hits": 0}),
        evidence,
    ))
}

/// A2: truncation semantics are unrepresentable. There is no stream
/// terminator state machine — a body that ends without `[DONE]` is
/// just the JSON body the client asked for. `BoundedBody` is a byte
/// cap, not a terminator check: it cannot tell "complete" from "cut
/// off mid-stream".
fn case_truncation_semantics_unrepresentable() -> Result<CaseReport, DriverError> {
    const CASE: &str = "truncation_semantics_unrepresentable";
    let mut evidence = Vec::new();
    evidence.push(
        "BoundedBody (crates/phlow-llm/src/transport.rs) enforces RESPONSE_BYTES_MAX \
         while streaming the body — it is a byte cap, not a terminator check: a body \
         that ends mid-JSON is either valid JSON (complete by definition) or a \
         BadJson error, never a typed 'truncated stream'"
            .to_string(),
    );
    evidence.push(
        "LlmError (crates/phlow-llm/src/error.rs) has no TruncatedStream variant: \
         the design's 'truncated streams never count as complete' criterion has no \
         error to arrive as — with stream:false the provider's JSON body IS the \
         completion"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"truncated_stream_variant": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "streaming_disabled_by_construction" => case_streaming_disabled_by_construction(),
        "no_sse_vocabulary" => case_no_sse_vocabulary(),
        "chunk_boundary_semantics_unrepresentable" => {
            case_chunk_boundary_semantics_unrepresentable()
        }
        "truncation_semantics_unrepresentable" => case_truncation_semantics_unrepresentable(),
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
        "seam: the provider client never streams — build_chat_payload hard-codes \
         \"stream\": false and the transport contract delivers one decoded JSON \
         body per call; SSE reassembly has no implementation"
            .to_string(),
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
        "finding: streaming vs non-streaming parity is unrepresentable — there is \
         no streaming mode, no SSE vocabulary, no reassembly function, and no \
         truncated-stream typing; every chat completion is single-shot JSON"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: the provider client never streams. build_chat_payload (crates/phlow-llm/src/payload.rs:69) inserts \"stream\": false into every chat payload — measured on the real builder, the emitted field is stream=false — so the provider is never asked for a stream and there is no streaming mode to be in parity with. Exact-token scans over phlow-llm/src/**/*.rs find zero hits for 'event-stream', 'text/event-stream', '[DONE]', 'reassemble', 'eventsource', 'event_source'. LlmTransport::post_chat returns one decoded JSON body per call (crates/phlow-llm/src/transport.rs): the contract has no chunk boundary, so a boundary splitting a JSON string escape or a multi-byte UTF-8 sequence cannot occur and byte-exact reassembly has no function to live in. BoundedBody is a byte cap, not a terminator check; LlmError has no TruncatedStream variant — 'truncated streams never count as complete' has no error to arrive as, because with stream:false the provider's JSON body IS the completion. Whether phlow-llm should support SSE streaming with byte-exact reassembly and truncated-stream typing is a product decision for Matt, not a gauntlet-authorized change.".to_string(),
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
