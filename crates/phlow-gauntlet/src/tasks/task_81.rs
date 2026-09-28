//! task-81: provider auth failure modes (rust).
//!
//! The design asks for the provider client's auth surface: every auth
//! failure shape (expired key, revoked key, malformed Authorization
//! header, wrong project/org, insufficient scope) must map to a typed,
//! distinct error, and 401s must never be retried.
//!
//! Seam recon (verified, not invented):
//! - The only provider client in the workspace is the Ollama backend
//!   (`crates/phlow-llm/src/transport.rs`, `OllamaBackend<T:
//!   LlmTransport>`). Ollama needs no auth: there is no `Authorization`
//!   header, no API key, no bearer token anywhere in `phlow-llm/src`
//!   (exact-token scans for `authorization`, `api_key`, `bearer`,
//!   `401`, `403`: zero hits each), and `OllamaConfig`
//!   (`crates/phlow-config/src/model.rs`) has no key field — only
//!   `base_url`, `model`, `temperature`, `context_length`,
//!   `timeout_secs`, `allow_remote`.
//! - There is no OpenAI or Anthropic client in any crate: the tool
//!   schemas in `phlow-tools` are OpenAI *function-calling schema
//!   shapes* (request payloads), not an API client.
//! - Auth failures therefore cannot be classified: `LlmError`
//!   (`crates/phlow-llm/src/error.rs`) has variants `BadRequest`,
//!   `ResponseTooLarge`, `BadJson`, `BadShape`, `Transport` — no
//!   auth variant. A provider 401/403 would arrive as the untyped
//!   `Transport(String)`.
//! - There is also no retry policy in the backend at all
//!   (`OllamaBackend::chat` makes exactly one `post_chat` call), so
//!   the design's "401s are never retried" holds by absence of retry
//!   machinery — but with no typed `auth_failed` error to hold it.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented on gauntlet
//! authority): whether phlow should gain API-key auth (e.g. for future
//! OpenAI/Anthropic providers) with typed auth-failure classification
//! and the no-retry-on-401 rule.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-81";
/// Human-readable name.
pub const NAME: &str = "provider auth failure modes";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_auth_surface",
    "auth_failures_untyped",
    "auth_failure_never_retried",
    "scope_error_untyped",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-81 driver itself (not of the code under test).
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
                write!(f, "task-81: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-81: probe {what} failed: {detail}")
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

/// V1: there is no auth surface to classify. Exact-token scans for the
/// auth vocabulary over `phlow-llm/src` find zero hits, and
/// `OllamaConfig` carries no key field.
fn case_no_auth_surface() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_auth_surface";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let mut total_hits = 0usize;
    for token in ["authorization", "api_key", "bearer", "401", "403"] {
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
    evidence.push(
        "OllamaConfig fields (crates/phlow-config/src/model.rs:180-187): base_url, model, \
         temperature, context_length, timeout_secs, allow_remote — no key field"
            .to_string(),
    );
    evidence.push(
        "Ollama needs no auth: the only provider client sends no credentials, so \
         there is no key material to classify, rotate, or leak (task-43's lesson is \
         vacuous here — and the fake transport records (method, url, payload) with \
         no Authorization header slot at all)"
            .to_string(),
    );
    if total_hits != 0 {
        return Ok(CaseReport::fail(
            CASE,
            format!("{total_hits} auth-vocabulary hit(s) — the absence finding is refuted"),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"auth_vocabulary_hits": 0}),
        evidence,
    ))
}

/// V2: auth failures are untyped. `LlmError` has no auth variant — a
/// provider 401 would arrive as the untyped `Transport(String)`.
fn case_auth_failures_untyped() -> Result<CaseReport, DriverError> {
    const CASE: &str = "auth_failures_untyped";
    let mut evidence = Vec::new();
    evidence.push(
        "LlmError variants (crates/phlow-llm/src/error.rs): BadRequest, ResponseTooLarge, \
         BadJson, BadShape, Transport — no AuthFailed, no ExpiredKey, no RevokedKey, \
         no MalformedAuth, no ScopeError"
            .to_string(),
    );
    let mut transport = FakeLlmTransport::new();
    transport.queue_reply(Err(LlmError::Transport("401 Unauthorized".to_owned())));
    let mut backend = backend(transport);
    let err = backend
        .chat(&chat_messages(), &[], None)
        .expect_err("a 401-flavored transport failure must surface as an error, not success");
    evidence.push(format!("scripted 401 -> backend.chat returned: {err}"));
    let typed = matches!(
        err,
        LlmError::BadRequest(_)
            | LlmError::ResponseTooLarge { .. }
            | LlmError::BadJson(_)
            | LlmError::BadShape(_)
    );
    if typed {
        return Ok(CaseReport::fail(
            CASE,
            format!("unexpected typed variant for a 401: {err}"),
            evidence,
        ));
    }
    match err {
        LlmError::Transport(detail) => {
            evidence.push(format!(
                "the 401 arrived as the UNTYPED Transport variant: {detail:?} — \
                 expired vs revoked vs malformed vs wrong-project are indistinguishable"
            ));
        }
        other => {
            return Ok(CaseReport::fail(
                CASE,
                format!("401 did not arrive as Transport: {other}"),
                evidence,
            ));
        }
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"typed_auth_variant": false}),
        evidence,
    ))
}

/// A1: a 401 on every call fails fast with zero retries — but the error
/// is untyped, not the design's `auth_failed`. The no-retry half of
/// the design holds by absence of retry machinery; the typed-error
/// half cannot hold.
fn case_auth_failure_never_retried() -> Result<CaseReport, DriverError> {
    const CASE: &str = "auth_failure_never_retried";
    let mut evidence = Vec::new();
    let mut transport = FakeLlmTransport::new();
    // Two scripted replies: a 401, then a success. If the backend
    // retried the 401, the second reply would be consumed and chat
    // would succeed — the attempt counter is the script consumption.
    transport.queue_reply(Err(LlmError::Transport("401 Unauthorized".to_owned())));
    transport.queue_reply(Ok(serde_json::json!({"choices": []})));
    let mut backend = backend(transport);
    let result = backend.chat(&chat_messages(), &[], None);
    match result {
        Ok(_) => {
            return Ok(CaseReport::fail(
                CASE,
                "backend retried the 401 and consumed the success reply".to_string(),
                evidence,
            ));
        }
        Err(LlmError::Transport(detail)) => {
            evidence.push(format!(
                "401 -> exactly one attempt (the queued success reply was never consumed), \
                 error: Transport({detail:?}) — zero retries, but untyped: no auth_failed class"
            ));
        }
        Err(other) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("401 surfaced as an unexpected variant: {other}"),
                evidence,
            ));
        }
    }
    evidence.push(
        "OllamaBackend::chat (crates/phlow-llm/src/transport.rs) makes exactly one \
         post_chat call: there is no retry loop to bound — the design's \
         'never retry 401' is satisfied vacuously, with no typed error to carry it"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"attempts": 1, "typed_auth_failed": false}),
        evidence,
    ))
}

/// A2: a 403 with an insufficient-scope message is indistinguishable
/// from any other transport failure — the operator cannot tell
/// whether to rotate the key or widen its scope.
fn case_scope_error_untyped() -> Result<CaseReport, DriverError> {
    const CASE: &str = "scope_error_untyped";
    let mut evidence = Vec::new();
    let mut transport = FakeLlmTransport::new();
    transport.queue_reply(Err(LlmError::Transport(
        "403 Forbidden: insufficient_scope for project proj_123".to_owned(),
    )));
    let mut backend = backend(transport);
    let err = backend
        .chat(&chat_messages(), &[], None)
        .expect_err("a 403-flavored failure must surface as an error");
    evidence.push(format!("scripted 403 insufficient_scope -> {err}"));
    match err {
        LlmError::Transport(detail) => {
            let names_scope = detail.contains("scope");
            evidence.push(format!(
                "the scope signal lives only in the free-text detail ({names_scope}): \
                 there is no ScopeError variant, so no distinct actionable message \
                 (widen scope) vs (rotate key) can be rendered"
            ));
        }
        other => {
            return Ok(CaseReport::fail(
                CASE,
                format!("403 did not arrive as Transport: {other}"),
                evidence,
            ));
        }
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"scope_variant": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_auth_surface" => case_no_auth_surface(),
        "auth_failures_untyped" => case_auth_failures_untyped(),
        "auth_failure_never_retried" => case_auth_failure_never_retried(),
        "scope_error_untyped" => case_scope_error_untyped(),
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
        "seam: the only provider client is OllamaBackend (crates/phlow-llm) — \
         Ollama needs no auth, so the design's auth-failure classification seam \
         has no implementation"
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
        "finding: 401s are never retried (the backend makes exactly one post_chat \
         call — no retry loop exists), but auth failures have no typed errors: \
         expired vs revoked vs malformed vs wrong-project vs insufficient-scope \
         all collapse into the untyped Transport(String)"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: the only provider client is OllamaBackend (crates/phlow-llm/src/transport.rs) and Ollama needs no auth — exact-token scans for 'authorization', 'api_key', 'bearer', '401', '403' over phlow-llm/src/**/*.rs find zero hits; OllamaConfig (crates/phlow-config/src/model.rs:180-187) has no key field; there is no OpenAI or Anthropic client in any crate (phlow-tools' OpenAI references are function-calling schema shapes, not an API client). LlmError (crates/phlow-llm/src/error.rs) has no auth variant: BadRequest, ResponseTooLarge, BadJson, BadShape, Transport — a provider 401/403 arrives as the untyped Transport(String), so expired vs revoked vs malformed vs wrong-project vs insufficient-scope are indistinguishable and no distinct actionable message can be rendered. The no-retry-on-401 half holds vacuously: OllamaBackend::chat makes exactly one post_chat call (measured: a scripted 401 followed by a queued success leaves chat returning the 401 error — one attempt, zero retries), but with no typed auth_failed error to carry the rule. Whether phlow should gain API-key auth with typed auth-failure classification is a product decision for Matt, not a gauntlet-authorized change.".to_string(),
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
