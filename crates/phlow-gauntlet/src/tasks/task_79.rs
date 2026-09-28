//! task-79: offline fallback (rust).
//!
//! The design asks for model resolution when the hub is unreachable:
//! offline must mean *pinned local cache or fail closed* — never a
//! silent model substitution. Offline with the pinned revision cached
//! runs with the cached model and labels the run record `offline:
//! true` plus the exact revision; offline without the pinned model
//! fails closed with `model_unavailable_offline` naming the missing
//! revision; a *different* cached revision must never be substituted
//! silently.
//!
//! Seam recon (verified, not invented):
//! - Model resolution in phlow is a config string passed verbatim to
//!   Ollama: `FlowConfig::model_for(role)` returns the role's
//!   override or `ollama.model` (`crates/phlow-config/src/model.rs`);
//!   `build_chat_payload` puts that string in the payload's `model`
//!   field unchanged. Model identity is an opaque `&str` — there is
//!   no revision type, no pin, no content hash anywhere in the model
//!   path.
//! - There is no model cache, no offline mode, and no `offline: true`
//!   run-record label. `OllamaConfig` exposes `model`, `base_url`,
//!   `timeout_secs`, `context_length` — no cache dir, no revision, no
//!   offline flag.
//! - Offline detection exists as a boolean: `OllamaBackend::is_available`
//!   returns false on any transport failure (refused, reset, timeout).
//!   When Ollama is unreachable, `chat` fails with the untyped
//!   `LlmError::Transport` — fail-closed (no fallback, no
//!   substitution), but not the design's typed
//!   `model_unavailable_offline`, and it names no revision because
//!   there is no revision to name.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented on gauntlet
//! authority): whether phlow should pin model revisions — a
//! content-hash-verified local cache, `offline: true` run-record
//! labeling, and a typed `model_unavailable_offline`. That is a new
//! product feature, not a bug fix.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_config::{FlowConfig, LoadOptions, ModelRole};
use phlow_llm::transport::OllamaBackend;
use phlow_llm::{FakeLlmTransport, LlmError};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-79";
/// Human-readable name.
pub const NAME: &str = "offline fallback";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "online_resolution",
    "offline_pinned_revision_absent",
    "offline_fails_closed_untyped",
    "no_silent_substitution_no_verification",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-79 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-79: cannot build fixture {what}: {detail}")
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
// Fixtures
// ---------------------------------------------------------------------------

/// Scratch dir unique to this process and case.
fn scratch_dir(case: &str) -> Result<PathBuf, DriverError> {
    let dir = std::env::temp_dir().join(format!("gauntlet-task-79-{}-{case}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| fixture_error("scratch dir", e))?;
    Ok(dir)
}

/// Load a real `FlowConfig` from a scratch TOML with the given
/// `[ollama] model` (task_54 pattern).
fn load_config_with_model(scratch: &Path, model: &str) -> Result<FlowConfig, DriverError> {
    let ws_dir = scratch.join("ws");
    std::fs::create_dir_all(&ws_dir).map_err(|e| fixture_error("workspace dir", e))?;
    let config_path = scratch.join("config.toml");
    let toml = format!(
        "workspace_dir = \"{}\"\n[ollama]\nmodel = \"{model}\"\n",
        ws_dir.display()
    );
    std::fs::write(&config_path, toml).map_err(|e| fixture_error("config write", e))?;
    phlow_config::load_config(&LoadOptions {
        config_path: Some(config_path),
        ..Default::default()
    })
    .map_err(|e| fixture_error("load_config", e))
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: the design's default — hub reachable. The configured model
/// resolves through `model_for`, the fake `/api/tags` lists it,
/// `is_available()` is true, and the real payload carries the model
/// name verbatim.
fn case_online_resolution() -> Result<CaseReport, DriverError> {
    const CASE: &str = "online_resolution";
    let mut evidence = Vec::new();
    let scratch = scratch_dir("online")?;
    let cfg = load_config_with_model(&scratch, "task-79-fixture")?;
    let resolved = cfg.model_for(ModelRole::Coder).to_string();
    evidence.push(format!(
        "FlowConfig::model_for(Coder) = {resolved:?} (role override empty -> ollama.model)"
    ));
    if resolved != "task-79-fixture" {
        return Ok(CaseReport::fail(
            CASE,
            format!("model_for resolved {resolved:?}, want \"task-79-fixture\""),
            evidence,
        ));
    }
    let mut transport = FakeLlmTransport::new();
    transport.queue_reply(Ok(serde_json::json!({
        "models": [{"name": "task-79-fixture"}],
    })));
    let mut backend = OllamaBackend::new(phlow_config::OllamaConfig::default(), transport);
    let available = backend.is_available();
    evidence.push(format!(
        "GET /api/tags answered -> is_available() = {available}"
    ));
    if !available {
        return Ok(CaseReport::fail(
            CASE,
            "backend not available with a healthy fake hub".to_string(),
            evidence,
        ));
    }
    // The payload path: the SAME configured string, verbatim.
    let ollama_cfg = phlow_config::OllamaConfig::default();
    let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
    let payload = phlow_llm::build_chat_payload(&ollama_cfg, &messages, &[], Some(&resolved))
        .map_err(|e| fixture_error("chat payload", e))?;
    let field = payload.get("model").cloned().unwrap_or(Value::Null);
    evidence.push(format!("real chat payload model field = {field}"));
    if field != serde_json::json!("task-79-fixture") {
        return Ok(CaseReport::fail(
            CASE,
            format!("payload model was {field}, not the configured name"),
            evidence,
        ));
    }
    evidence.push(
        "online resolution works: config string -> model_for -> payload, verbatim; \
         no revision, no hash, no cache involved"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"resolved": resolved, "available": true}),
        evidence,
    ))
}

/// V2: the design's "offline with the pinned revision cached" — runs
/// with the cached model, run record notes `offline: true` + exact
/// revision. None of the machinery exists: model identity is an
/// opaque string, `set_model` validates only non-emptiness (a fake
/// `@sha256:` suffix is accepted as mere characters), and there is no
/// cache dir, no revision field, no offline label.
fn case_offline_pinned_revision_absent() -> Result<CaseReport, DriverError> {
    const CASE: &str = "offline_pinned_revision_absent";
    let mut evidence = Vec::new();
    let mut ollama_cfg = phlow_config::OllamaConfig::default();
    // A "pinned revision" is just characters in the opaque string:
    // accepted without any verification.
    ollama_cfg
        .set_model("task-79-fixture@sha256:deadbeef".to_string())
        .map_err(|e| fixture_error("set_model", e))?;
    evidence.push(format!(
        "set_model(\"task-79-fixture@sha256:deadbeef\") accepted: model = {:?} — \
         the revision suffix is unverified characters, not a pin",
        ollama_cfg.model()
    ));
    if ollama_cfg.model() != "task-79-fixture@sha256:deadbeef" {
        return Ok(CaseReport::fail(
            CASE,
            "set_model did not round-trip the revision string".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "OllamaConfig exposes model / base_url / timeout_secs / context_length — \
         no cache dir, no revision field, no offline flag; FlowConfig::model_for \
         returns &str — there is no revision type to pin"
            .to_string(),
    );
    evidence.push(
        "no content-hash verification exists anywhere in the model path; no \
         `offline: true` run-record label exists — the design's offline-cached \
         run is unrepresentable"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"revision_pinning_exists": false, "offline_label_exists": false}),
        evidence,
    ))
}

/// A1: offline WITHOUT the pinned model cached — the design wants
/// fail-closed with `model_unavailable_offline` naming the missing
/// revision. Reality: the fake hub refuses, `is_available()` is
/// false, and `chat` fails with the untyped `LlmError::Transport` —
/// fail-closed (no fallback, no substitution), but untyped and
/// revision-blind.
fn case_offline_fails_closed_untyped() -> Result<CaseReport, DriverError> {
    const CASE: &str = "offline_fails_closed_untyped";
    let mut evidence = Vec::new();
    let mut transport = FakeLlmTransport::new();
    transport.queue_reply(Err(LlmError::Transport(
        "connection refused: 127.0.0.1:11434".to_string(),
    )));
    // A second failure queued for the chat call itself.
    transport.queue_reply(Err(LlmError::Transport(
        "connection refused: 127.0.0.1:11434".to_string(),
    )));
    let mut backend = OllamaBackend::new(phlow_config::OllamaConfig::default(), transport);
    let available = backend.is_available();
    evidence.push(format!(
        "GET /api/tags refused -> is_available() = {available} (the offline detector)"
    ));
    if available {
        return Ok(CaseReport::fail(
            CASE,
            "backend available despite refused hub".to_string(),
            evidence,
        ));
    }
    let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
    match backend.chat(&messages, &[], Some("task-79-fixture")) {
        Ok(_) => {
            return Ok(CaseReport::fail(
                CASE,
                "chat SUCCEEDED with the hub down: not fail-closed".to_string(),
                evidence,
            ));
        }
        Err(e) => {
            evidence.push(format!(
                "chat with hub down -> Err({e:?}); Display: \"{e}\""
            ));
            if !matches!(e, LlmError::Transport(_)) {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("unexpected error variant: {e:?}"),
                    evidence,
                ));
            }
            let display = e.to_string();
            if display.contains("model_unavailable_offline") {
                return Ok(CaseReport::fail(
                    CASE,
                    "a typed model_unavailable_offline EXISTS — finding refuted".to_string(),
                    evidence,
                ));
            }
            evidence.push(
                "fail-closed (no fallback model, no silent substitution — the error \
                 propagates), but UNTYPED: LlmError::Transport, not \
                 model_unavailable_offline, and it names no revision (there is no \
                 revision to name)"
                    .to_string(),
            );
        }
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"available": false, "typed_offline_error": false}),
        evidence,
    ))
}

/// A2: the adversarial-in-V "helpful swap" — a DIFFERENT revision of
/// the same model must never be substituted silently. phlow never
/// substitutes: the configured string reaches the payload verbatim.
/// But phlow also cannot DETECT a substitution: with no content hash,
/// if the provider serves a different revision under the same tag,
/// phlow is blind.
fn case_no_silent_substitution_no_verification() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_silent_substitution_no_verification";
    let mut evidence = Vec::new();
    let ollama_cfg = phlow_config::OllamaConfig::default();
    let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
    // A "different revision" of the same model: distinct strings.
    for configured in ["task-79-fixture@rev-a", "task-79-fixture@rev-b"] {
        let payload = phlow_llm::build_chat_payload(&ollama_cfg, &messages, &[], Some(configured))
            .map_err(|e| fixture_error("chat payload", e))?;
        let field = payload.get("model").cloned().unwrap_or(Value::Null);
        if field != serde_json::json!(configured) {
            return Ok(CaseReport::fail(
                CASE,
                format!("payload rewrote the model string: {field}"),
                evidence,
            ));
        }
        evidence.push(format!(
            "configured {configured:?} -> payload model = {field}: verbatim, never swapped"
        ));
    }
    evidence.push(
        "phlow performs no substitution — but also no verification: the model \
         string is opaque, so a provider-side tag move (same tag, different \
         bytes) is undetectable by phlow. The design's \"cached-revision \
         verification by content hash, not directory name\" does not exist"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"substituted": false, "verified": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "online_resolution" => case_online_resolution(),
        "offline_pinned_revision_absent" => case_offline_pinned_revision_absent(),
        "offline_fails_closed_untyped" => case_offline_fails_closed_untyped(),
        "no_silent_substitution_no_verification" => case_no_silent_substitution_no_verification(),
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
        "recon: model resolution is a config string passed verbatim to Ollama — FlowConfig::model_for(role) returns the role override or ollama.model; build_chat_payload puts it in the payload's model field unchanged".to_string(),
        "recon: no model cache, no revision pinning, no content-hash verification, no offline mode, no `offline: true` run-record label — OllamaConfig exposes model/base_url/timeout_secs/context_length only".to_string(),
        "recon: offline detection is boolean — OllamaBackend::is_available() is false on any transport failure; chat then fails with untyped LlmError::Transport (fail-closed, but not model_unavailable_offline and revision-blind)".to_string(),
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
        "finding: offline means transport-error, not a mode — there is no pinned cache to fall back to, no offline labeling, and no typed offline error; the opaque model string is never substituted by phlow but also never verified".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: phlow's model resolution is a config string passed verbatim to Ollama (FlowConfig::model_for -> build_chat_payload's model field, unchanged) — there is no model cache, no revision pinning (set_model accepts any non-empty string; a @sha256: suffix is unverified characters), no content-hash verification anywhere in the model path, no offline mode, and no `offline: true` run-record label. Offline detection is boolean (OllamaBackend::is_available() false on any transport failure) and chat fails closed with the untyped LlmError::Transport — no fallback and no silent substitution, but not the design's typed model_unavailable_offline, and it names no revision because there is no revision to name. A provider-side tag move (same tag, different bytes) is undetectable by phlow. Whether phlow should pin model revisions (content-hash-verified local cache, offline:true labeling, typed model_unavailable_offline) is a product decision for Matt, not a gauntlet-authorized change.".to_string(),
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
