//! task-82: provider rate-limit protocol compliance (rust).
//!
//! The design asks for the provider client's rate-limit protocol:
//! honor `Retry-After`, classify 429 / 503 / 401 into distinct code
//! paths, clamp absurd waits, bound infinite-429 sequences, and keep
//! per-provider quota state that never leaks across providers.
//!
//! Seam recon (verified, not invented):
//! - `phlow-llm/src` has zero rate-limit vocabulary: exact-token
//!   scans for `429`, `retry-after`, `retry_after`, `backoff`,
//!   `rate_limit`, `ratelimit` find zero hits. `LlmError`
//!   (`crates/phlow-llm/src/error.rs`) has no rate-limit variant.
//! - The `LlmTransport` trait
//!   (`crates/phlow-llm/src/transport.rs`) passes `(base_url,
//!   payload, timeout)` — no header plumbing: a `Retry-After` header
//!   could not reach the backend even if the transport read one.
//! - The backend has no retry loop at all: `OllamaBackend::chat`
//!   makes exactly one `post_chat` call. Infinite 429s therefore
//!   terminate trivially per call (each call fails once), but there
//!   is no backoff, no give-up counter, and no quota state — retry
//!   policy is the caller's problem, unbounded.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented on gauntlet
//! authority): whether phlow-llm should speak the 429/Retry-After
//! protocol — clamped waits, bounded give-up, per-provider quota —
//! instead of the current single-attempt client.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-82";
/// Human-readable name.
pub const NAME: &str = "provider rate-limit protocol compliance";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_429_classification",
    "no_retry_after_parsing",
    "no_bounded_retry",
    "no_per_provider_quota_state",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-82 driver itself (not of the code under test).
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
                write!(f, "task-82: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-82: probe {what} failed: {detail}")
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
use phlow_llm::error::LlmError;
use phlow_llm::transport::{FakeLlmTransport, OllamaBackend};

fn backend(transport: FakeLlmTransport) -> OllamaBackend<FakeLlmTransport> {
    OllamaBackend::new(OllamaConfig::default(), transport)
}

fn chat_messages() -> Vec<Value> {
    vec![serde_json::json!({"role": "user", "content": "hi"})]
}

/// V1: 429 has no classification. The scan finds zero `429` hits in
/// `phlow-llm/src`, and a scripted 429-flavored failure arrives as the
/// untyped `Transport(String)` — the same variant as a refused
/// connection.
fn case_no_429_classification() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_429_classification";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let hits = scan_sources(&root, "phlow-llm", "429")?;
    evidence.push(format!(
        "exact-token scan for '429' over phlow-llm/src/**/*.rs: {} hit(s)",
        hits.len()
    ));
    for hit in &hits {
        evidence.push(format!("  unexpected hit: {hit}"));
    }
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            "429 vocabulary exists — the absence finding is refuted".to_string(),
            evidence,
        ));
    }
    let mut transport = FakeLlmTransport::new();
    transport.queue_reply(Err(LlmError::Transport("429 Too Many Requests".to_owned())));
    let mut backend = backend(transport);
    let err = backend
        .chat(&chat_messages(), &[], None)
        .expect_err("a 429-flavored failure must surface as an error");
    match err {
        LlmError::Transport(detail) => {
            evidence.push(format!(
                "scripted 429 -> untyped Transport({detail:?}): no distinct 429 code path, \
                 no Retry-After read, no backoff scheduled"
            ));
        }
        other => {
            return Ok(CaseReport::fail(
                CASE,
                format!("429 did not arrive as Transport: {other}"),
                evidence,
            ));
        }
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"status_429_hits": 0, "typed_429_variant": false}),
        evidence,
    ))
}

/// V2: `Retry-After` is never parsed — the trait signature cannot even
/// carry it. `LlmTransport::post_chat` takes `(base_url, payload,
/// timeout)`: there is no header slot, so even a transport that read
/// the header could not deliver it to the backend.
fn case_no_retry_after_parsing() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_retry_after_parsing";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let mut total_hits = 0usize;
    for token in ["retry-after", "retry_after"] {
        let hits = scan_sources(&root, "phlow-llm", token)?;
        evidence.push(format!(
            "exact-token scan for '{token}' over phlow-llm/src/**/*.rs: {} hit(s)",
            hits.len()
        ));
        total_hits += hits.len();
    }
    evidence.push(
        "LlmTransport::post_chat(base_url: &str, payload: &Map<String, Value>, timeout: Duration) \
         (crates/phlow-llm/src/transport.rs) — no header parameter, no header return: \
         Retry-After has no path from the wire to any decision"
            .to_string(),
    );
    if total_hits != 0 {
        return Ok(CaseReport::fail(
            CASE,
            format!("{total_hits} Retry-After hit(s) — the absence finding is refuted"),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"retry_after_hits": 0}),
        evidence,
    ))
}

/// A1: there is no bounded retry — because there is no retry at all.
/// A provider that 429s forever meets a client that fails once per
/// call: each call is exactly one attempt. The design's bounded
/// give-up (`rate_limit_exhausted`, capped attempts) is
/// unrepresentable: the give-up counter would live in a retry loop
/// that does not exist.
fn case_no_bounded_retry() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_bounded_retry";
    let mut evidence = Vec::new();
    // A provider that 429s forever: five calls, five 429s. The
    // scripted-reply consumption is the attempt counter — if the
    // backend retried internally, one call would consume many replies.
    let mut transport = FakeLlmTransport::new();
    for _ in 0..5 {
        transport.queue_reply(Err(LlmError::Transport(
            "429 Too Many Requests; Retry-After: 3600".to_owned(),
        )));
    }
    let mut backend = backend(transport);
    for attempt in 1..=5usize {
        let err = backend
            .chat(&chat_messages(), &[], None)
            .expect_err("the 429 storm must keep failing");
        if !matches!(err, LlmError::Transport(_)) {
            return Ok(CaseReport::fail(
                CASE,
                format!("attempt {attempt}: 429 did not arrive as Transport: {err}"),
                evidence,
            ));
        }
    }
    evidence.push(
        "5 calls against an always-429 provider consumed exactly 5 scripted replies: \
         one attempt per call, zero internal retries — there is no retry loop in \
         OllamaBackend::chat to clamp, bound, or exhaust"
            .to_string(),
    );
    evidence.push(
        "consequence for the design: 'Retry-After: 3600 clamped to a named max wait, \
         then rate_limit_wait_exceeded' and 'infinite-429 terminates bounded with \
         rate_limit_exhausted' both need a wait-and-retry state machine; none exists \
         — termination-per-call is trivial, but backoff, clamping, and give-up are \
         the caller's unbounded problem"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"calls": 5, "attempts_per_call": 1, "retry_loop": false}),
        evidence,
    ))
}

/// A2: per-provider quota state does not exist. The scan finds zero
/// `quota` hits, and two backends share nothing — there is no counter
/// to leak across providers because there is no counter at all.
fn case_no_per_provider_quota_state() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_per_provider_quota_state";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let hits = scan_sources(&root, "phlow-llm", "quota")?;
    evidence.push(format!(
        "exact-token scan for 'quota' over phlow-llm/src/**/*.rs: {} hit(s)",
        hits.len()
    ));
    for hit in &hits {
        evidence.push(format!("  unexpected hit: {hit}"));
    }
    if !hits.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            "quota vocabulary exists — the absence finding is refuted".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "OllamaBackend holds (cfg, base_url, transport) only \
         (crates/phlow-llm/src/transport.rs): no counters, no per-provider map, \
         no shared state between backends — 'per-provider counters never leak \
         across providers' is vacuous with no counters"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"quota_hits": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_429_classification" => case_no_429_classification(),
        "no_retry_after_parsing" => case_no_retry_after_parsing(),
        "no_bounded_retry" => case_no_bounded_retry(),
        "no_per_provider_quota_state" => case_no_per_provider_quota_state(),
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
        "seam: the provider client (OllamaBackend, crates/phlow-llm) speaks no \
         rate-limit protocol — no 429 classification, no Retry-After parsing, no \
         retry loop, no per-provider quota state"
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
        "finding: the client is single-attempt — 429/503/401 are not distinct code \
         paths, Retry-After has no wire-to-decision path, and there is no retry \
         loop to clamp or bound; per-provider quota state does not exist"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: phlow-llm speaks no rate-limit protocol. Exact-token scans over phlow-llm/src/**/*.rs find zero hits for '429', 'retry-after', 'retry_after', and 'quota'; LlmError (crates/phlow-llm/src/error.rs) has no rate-limit variant, so a scripted 429 arrives as the untyped Transport(String) — the same variant as a refused connection, with no distinct 429 / 503 / 401 code path. LlmTransport::post_chat takes (base_url, payload, timeout) with no header slot (crates/phlow-llm/src/transport.rs): Retry-After could not reach the backend even if a transport read it. OllamaBackend::chat makes exactly one post_chat call — measured: 5 calls against an always-429 fake consumed exactly 5 scripted replies, one attempt per call, zero internal retries — so the design's clamped waits, bounded give-up (rate_limit_exhausted), and per-provider quota counters are unrepresentable: the retry loop they would live in does not exist. Whether phlow-llm should speak the 429/Retry-After protocol (honored-and-clamped waits, bounded give-up, per-provider quota) is a product decision for Matt, not a gauntlet-authorized change.".to_string(),
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
