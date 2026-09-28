//! task-84: provider outage failover (rust).
//!
//! The design asks for the provider router / failover policy:
//! failover goes only to pre-approved secondaries (never discovered
//! or improvised), a local-only workload must never fail over to a
//! cloud provider (data exfiltration by routing), failover decisions
//! are logged with cause, and a down failover target is bounded and
//! typed (no cascade loop).
//!
//! Seam recon (verified, not invented):
//! - There is no provider router in the workspace: exact-token
//!   scans for `failover` / `fail_over` over every product crate's
//!   `src/**/*.rs` find zero hits.
//! - The client is single-provider by construction: `OllamaBackend`
//!   holds exactly one `base_url`, and `OllamaConfig`
//!   (`crates/phlow-config/src/model.rs:180-187`) has no secondary
//!   field — only `base_url`, `model`, `temperature`,
//!   `context_length`, `timeout_secs`, `allow_remote`.
//! - An outage therefore fails closed by construction: `chat`
//!   returns the transport error to the caller, `is_available()`
//!   returns false. There is no secondary to fail over to, no
//!   decision to log, and no routing policy to violate.
//! - Partial truth, recorded honestly: with exactly one configured
//!   URL (default loopback `http://127.0.0.1:11434`,
//!   `allow_remote: false`), no cloud request CAN be emitted —
//!   measured at the HTTP layer via a recording transport. The
//!   design's local-only invariant holds by architecture, but the
//!   routing policy the design asks to verify has no implementation.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented on gauntlet
//! authority): whether phlow should gain multi-provider routing with
//! a failover policy (pre-approved secondaries, logged decisions,
//! local-only data-boundary enforcement).

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-84";
/// Human-readable name.
pub const NAME: &str = "provider outage failover";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "single_provider_architecture",
    "no_failover_vocabulary",
    "outage_is_not_rerouted",
    "local_only_invariant_by_architecture",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-84 driver itself (not of the code under test).
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
                write!(f, "task-84: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-84: probe {what} failed: {detail}")
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

/// Exact-token (case-insensitive) hits for `token` over every product
/// crate's `src/**/*.rs` — the gauntlet crate itself excluded. Bounded
/// like task_48.
fn scan_workspace(root: &Path, token: &str) -> Result<Vec<String>, DriverError> {
    let excluded = excluded_crate_root();
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
                if path != excluded {
                    stack.push(path);
                }
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
// Recording transport: the HTTP-layer assertion harness
// ---------------------------------------------------------------------------

use phlow_config::OllamaConfig;
use phlow_llm::error::LlmError;
use phlow_llm::transport::{LlmTransport, OllamaBackend};

/// A [`LlmTransport`] that records every requested URL and fails every
/// call with the scripted error: the HTTP-layer eye for the outage
/// and local-only cases. Single-threaded by construction (the driver
/// never shares it across threads).
#[derive(Debug)]
struct RecordingTransport {
    urls: Rc<RefCell<Vec<String>>>,
    failure: String,
    closed: bool,
}

impl RecordingTransport {
    fn new(urls: Rc<RefCell<Vec<String>>>, failure: &str) -> Self {
        Self {
            urls,
            failure: failure.to_string(),
            closed: false,
        }
    }
}

impl LlmTransport for RecordingTransport {
    fn post_chat(
        &mut self,
        base_url: &str,
        _payload: &Map<String, Value>,
        _timeout: Duration,
    ) -> Result<Value, LlmError> {
        self.urls
            .borrow_mut()
            .push(format!("{base_url}/v1/chat/completions"));
        Err(LlmError::Transport(self.failure.clone()))
    }

    fn get_tags(&mut self, base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
        self.urls.borrow_mut().push(format!("{base_url}/api/tags"));
        Err(LlmError::Transport(self.failure.clone()))
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

fn chat_messages() -> Vec<Value> {
    vec![serde_json::json!({"role": "user", "content": "hi"})]
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: single-provider architecture. The backend holds exactly one
/// `base_url`; `OllamaConfig` has no secondary field.
fn case_single_provider_architecture() -> Result<CaseReport, DriverError> {
    const CASE: &str = "single_provider_architecture";
    let mut evidence = Vec::new();
    let urls = Rc::new(RefCell::new(Vec::new()));
    let transport = RecordingTransport::new(Rc::clone(&urls), "connection refused");
    let backend = OllamaBackend::new(OllamaConfig::default(), transport);
    evidence.push(format!(
        "OllamaBackend::base_url() = {:?} — exactly one provider URL",
        backend.base_url()
    ));
    evidence.push(
        "OllamaConfig fields (crates/phlow-config/src/model.rs:180-187): base_url, model, \
         temperature, context_length, timeout_secs, allow_remote — no secondary_url, \
         no failover target list"
            .to_string(),
    );
    if backend.base_url().is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            "backend base_url is empty".to_string(),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"provider_urls": 1}),
        evidence,
    ))
}

/// V2: no failover vocabulary in the workspace. Exact-token scans for
/// `failover` / `fail_over` over every product crate's
/// `src/**/*.rs` find zero hits.
fn case_no_failover_vocabulary() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_failover_vocabulary";
    let mut evidence = Vec::new();
    let root = workspace_root()?;
    let mut total_hits = 0usize;
    for token in ["failover", "fail_over"] {
        let hits = scan_workspace(&root, token)?;
        evidence.push(format!(
            "exact-token scan for '{token}' over crates/*/src/**/*.rs: {} hit(s)",
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
            format!("{total_hits} failover hit(s) — the absence finding is refuted"),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"failover_vocabulary_hits": 0}),
        evidence,
    ))
}

/// A1: an outage is not rerouted — it fails closed. The scripted
/// outage (connection refused) surfaces as the caller's error after
/// exactly one request; there is no secondary to fail over to and no
/// failover decision logged (nothing logs it).
fn case_outage_is_not_rerouted() -> Result<CaseReport, DriverError> {
    const CASE: &str = "outage_is_not_rerouted";
    let mut evidence = Vec::new();
    let urls = Rc::new(RefCell::new(Vec::new()));
    let transport = RecordingTransport::new(Rc::clone(&urls), "connection refused");
    let mut backend = OllamaBackend::new(OllamaConfig::default(), transport);
    let err = backend
        .chat(&chat_messages(), &[], None)
        .expect_err("the scripted outage must surface as an error");
    let recorded = urls.borrow();
    evidence.push(format!("scripted outage -> chat returned: {err}"));
    evidence.push(format!(
        "HTTP layer recorded {} request(s): {:?}",
        recorded.len(),
        recorded.as_slice()
    ));
    if recorded.len() != 1 {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "expected exactly 1 request (no reroute target exists), saw {}",
                recorded.len()
            ),
            evidence,
        ));
    }
    // Fail-closed: the same outage makes the backend unavailable, and
    // the error propagates — no silent switch, because there is no
    // switch at all.
    drop(recorded);
    let available = backend.is_available();
    let recorded = urls.borrow();
    evidence.push(format!(
        "is_available() after outage = {available}; total recorded requests: {}",
        recorded.len()
    ));
    if available {
        return Ok(CaseReport::fail(
            CASE,
            "backend reports available during a scripted outage".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "fail-closed holds by construction: the error propagates to the caller and \
         is_available() is false — but no failover decision is logged with cause \
         (there is no failover machinery to log)"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"requests_during_outage": 1, "rerouted": false}),
        evidence,
    ))
}

/// A2: the local-only invariant holds by architecture, not by policy.
/// With the default config the only URL ever requested is the
/// loopback base URL — measured at the HTTP layer. But the design's
/// routing policy (workload marked local-only must never fail over
/// to cloud) has no implementation: there is no workload marking,
/// no router, no cloud secondary to refuse.
fn case_local_only_invariant_by_architecture() -> Result<CaseReport, DriverError> {
    const CASE: &str = "local_only_invariant_by_architecture";
    let mut evidence = Vec::new();
    let cfg = OllamaConfig::default();
    evidence.push(format!(
        "default OllamaConfig: base_url={:?}, allow_remote={}",
        cfg.base_url(),
        cfg.allow_remote()
    ));
    let urls = Rc::new(RefCell::new(Vec::new()));
    let transport = RecordingTransport::new(Rc::clone(&urls), "connection refused");
    let mut backend = OllamaBackend::new(OllamaConfig::default(), transport);
    let _ = backend.chat(&chat_messages(), &[], None);
    let _ = backend.is_available();
    let recorded = urls.borrow();
    evidence.push(format!(
        "HTTP layer recorded {} request(s) across an outage + availability check: {:?}",
        recorded.len(),
        recorded.as_slice()
    ));
    let cloud_hit = recorded
        .iter()
        .any(|u| !u.starts_with("http://127.0.0.1") && !u.starts_with("http://localhost"));
    if cloud_hit {
        return Ok(CaseReport::fail(
            CASE,
            format!("a non-loopback URL was requested: {recorded:?}"),
            evidence,
        ));
    }
    evidence.push(
        "no cloud request was emitted: with exactly one configured URL there is \
         nothing to discover and nothing to improvise — the design's \
         'local-only workloads never leave the machine' holds vacuously"
            .to_string(),
    );
    evidence.push(
        "but the routing POLICY is absent: no workload is marked local-only, no \
         router checks the marking, no cloud secondary exists to be refused — the \
         data-boundary invariant the design asks to verify has no implementation"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"cloud_requests": 0, "routing_policy": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "single_provider_architecture" => case_single_provider_architecture(),
        "no_failover_vocabulary" => case_no_failover_vocabulary(),
        "outage_is_not_rerouted" => case_outage_is_not_rerouted(),
        "local_only_invariant_by_architecture" => case_local_only_invariant_by_architecture(),
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
        "seam: no provider router exists — the client is single-provider \
         (OllamaBackend, one base_url); the failover policy the design asks for \
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
        "finding: outages fail closed (error propagates, is_available false) and \
         no cloud request can be emitted (one loopback URL, no discovery), but \
         pre-approved secondaries, logged failover decisions, and the local-only \
         routing policy do not exist"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: there is no provider router in the workspace. Exact-token scans for 'failover' / 'fail_over' over crates/*/src/**/*.rs find zero hits; OllamaBackend holds exactly one base_url (measured via base_url()) and OllamaConfig (crates/phlow-config/src/model.rs:180-187) has no secondary field — only base_url, model, temperature, context_length, timeout_secs, allow_remote. Measured at the HTTP layer with a recording transport: a scripted outage (connection refused) makes chat return the error after exactly 1 recorded request — the outage fails closed (error propagates, is_available() false) with no reroute, no failover decision, and nothing logged with cause, because there is no secondary and no router. The design's local-only invariant holds vacuously: across the outage and availability check the only URLs ever requested were the loopback base_url (0 cloud requests) — with exactly one configured URL there is nothing to discover or improvise — but the routing POLICY the design asks to verify (workload marked local-only, router refuses cloud failover) has no implementation: no workload marking, no router, no cloud secondary to refuse. Whether phlow should gain multi-provider routing with a failover policy (pre-approved secondaries, logged decisions, local-only data-boundary enforcement) is a product decision for Matt, not a gauntlet-authorized change.".to_string(),
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
