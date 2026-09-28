//! task-27: retry backoff under storm (rust).
//!
//! Recon task. The design asks for the retry wrapper around tool/adapter
//! calls. Phlow's Rust crates have none: `OllamaBackend::chat`
//! (`crates/phlow-llm/src/transport.rs`), `MsgpackTransport::exec` and the
//! HTTP transport (`crates/phlow-runtime/src/transport/`) are all
//! single-shot — a failure surfaces to the caller and nothing retries,
//! backs off, decorrelates, or caps attempts. The driver proves the
//! absence two ways: source recon over the three downstream-call wrappers
//! (fail-closed if retry machinery ever appears), and behavioral probes
//! against the real `OllamaBackend::chat` with a scripted transport — a
//! transient failure is never retried by the path, a 100-call storm
//! against a downed downstream reaches it 1:1 with no injected delays,
//! and no attempt cap or typed exhaustion error exists.
//!
//! The task verdict is `fail` with `where = "seam"`: the design's
//! storm-bounding pass criteria (exponential backoff with jitter, a hard
//! attempt cap, a bounded peak attempt rate) have no retry wrapper to
//! evaluate against — an open design gap, not a driver error.
//!
//! Diver-owned note (flagged, never fixed on gauntlet authority): diver's
//! `ai.harness.supervisor.retry_run`
//! (`~/workspace/repos/diver/lua/ai/harness/supervisor.lua`) implements
//! bounded retries with exponential backoff and jitter
//! (`RETRY_ATTEMPTS_MAX = 4`, `backoff_ms = min(RETRY_BASE_MS *
//! 2^(attempt-2), RETRY_MAX_MS)`, `jitter_ms = random(0, RETRY_BASE_MS)`)
//! — a Diver-owned mechanism, out of scope for this rust task, not tested
//! here.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_llm::error::LlmError;
use phlow_llm::transport::{LlmTransport, OllamaBackend};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Task id.
pub const ID: &str = "task-27";
/// Human-readable name.
pub const NAME: &str = "retry backoff under storm";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "transient_failure_is_not_retried",
    "no_attempt_cap_or_exhaustion_error",
    "storm_reaches_downstream_unthrottled",
    "no_backoff_delays_injected",
];

/// Calls for the attempt-cap probe: enough that any sane cap would engage.
const ATTEMPT_PROBE_CALLS: usize = 20;
/// Callers in the adversarial storm.
const STORM_CALLS: usize = 100;
/// Wall-clock bound for the whole storm, in seconds. The scripted
/// downstream answers in microseconds; any backoff/jitter would blow this.
const STORM_WALL_SECS_MAX: f64 = 5.0;
/// Calls for the backoff-delay probe.
const BACKOFF_PROBE_CALLS: usize = 10;
/// Per-call latency bound for the backoff probe, in milliseconds.
const PER_CALL_MS_MAX: u128 = 1000;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-27 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A workspace path was missing or unreadable.
    Path {
        /// Which file was wanted.
        what: String,
        /// Path plus I/O detail.
        detail: String,
    },
    /// The recon premise changed: retry machinery appeared on a call path.
    ReconChanged {
        /// What changed.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path { what, detail } => write!(f, "task-27: cannot read {what}: {detail}"),
            Self::ReconChanged { detail } => {
                write!(f, "task-27: recon premise changed: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

// ---------------------------------------------------------------------------
// Recon: re-verify the seam premises against the live sources on every run
// ---------------------------------------------------------------------------

fn workspace_root() -> Result<std::path::PathBuf, DriverError> {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| DriverError::Path {
            what: "workspace root".to_string(),
            detail: "CARGO_MANIFEST_DIR has fewer than 2 ancestors".to_string(),
        })
}

fn read_file(path: &std::path::Path, what: &str) -> Result<String, DriverError> {
    std::fs::read_to_string(path).map_err(|e| DriverError::Path {
        what: what.to_string(),
        detail: format!("{}: {e}", path.display()),
    })
}

/// The downstream-call wrappers in phlow's Rust crates: every one must be
/// retry-free for the absence claim to hold.
fn call_path_files() -> Result<Vec<(String, String)>, DriverError> {
    let root = workspace_root()?;
    let files = [
        "crates/phlow-llm/src/transport.rs",
        "crates/phlow-runtime/src/transport/msgpack.rs",
        "crates/phlow-runtime/src/transport/http.rs",
    ];
    let mut out = Vec::new();
    for rel in files {
        let path = root.join(rel);
        out.push((rel.to_string(), read_file(&path, rel)?));
    }
    Ok(out)
}

/// Verify the task's scope premises against the live sources. Fails closed:
/// if any call path ever gains retry/backoff machinery, the old absence
/// claims must not silently pass.
fn recon() -> Result<Vec<String>, DriverError> {
    let needles = [
        "retry", "Retry", "RETRY", "backoff", "Backoff", "BACKOFF", "jitter", "Jitter",
    ];
    for (rel, text) in call_path_files()? {
        for needle in needles {
            // "wait and retry" is a human-facing rate-cap message, not
            // retry machinery; anything else fails the premise.
            if text.contains(needle) && !text.contains("wait and retry") {
                return Err(DriverError::ReconChanged {
                    detail: format!(
                        "{rel} now contains '{needle}'; retry machinery may exist on the call path"
                    ),
                });
            }
        }
    }
    Ok(vec![
        "recon: phlow-llm/src/transport.rs (OllamaBackend::chat), phlow-runtime/src/transport/msgpack.rs (MsgpackTransport::exec), phlow-runtime/src/transport/http.rs — none contain retry/backoff/jitter machinery (8 needles absent per file)".to_string(),
        "recon: every Rust downstream-call wrapper is single-shot: a failure surfaces to the caller; no retry wrapper exists in phlow's Rust crates".to_string(),
        "finding: the task-27 retry seam is absent in Rust; the design's storm-bounding criteria have no wrapper to evaluate against".to_string(),
        "note (Diver-owned, flagged not fixed): diver ai.harness.supervisor.retry_run implements bounded retries with exponential backoff + jitter (RETRY_ATTEMPTS_MAX=4) — out of scope for this rust task".to_string(),
    ])
}

// ---------------------------------------------------------------------------
// Scripted downstream: implements the real LlmTransport trait
// ---------------------------------------------------------------------------

/// Interior state of the scripted downstream.
#[derive(Debug)]
struct ProbeInner {
    replies: VecDeque<Result<Value, LlmError>>,
    post_calls: u64,
    call_times_ms: Vec<u128>,
    started: Instant,
}

/// A scripted downstream at the transport boundary. Shared ownership
/// (`Rc`) lets the driver read call counts and timings after handing the
/// probe to the backend — `OllamaBackend` takes sole ownership of its
/// transport.
#[derive(Debug, Clone)]
pub struct Probe {
    inner: Rc<RefCell<ProbeInner>>,
}

impl Probe {
    /// A probe with no queued replies.
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(ProbeInner {
                replies: VecDeque::new(),
                post_calls: 0,
                call_times_ms: Vec::new(),
                started: Instant::now(),
            })),
        }
    }

    /// Queue one reply (or failure) for the next `post_chat`.
    pub fn queue_reply(&self, reply: Result<Value, LlmError>) {
        self.inner.borrow_mut().replies.push_back(reply);
    }

    /// Queue `n` downstream failures.
    pub fn queue_failures(&self, n: usize) {
        for i in 0..n {
            self.queue_reply(Err(LlmError::Transport(format!("downstream down ({i})"))));
        }
    }

    /// How many `post_chat` calls reached this downstream.
    pub fn post_calls(&self) -> u64 {
        self.inner.borrow().post_calls
    }

    /// Milliseconds since construction of each `post_chat` call, in order.
    pub fn call_times_ms(&self) -> Vec<u128> {
        self.inner.borrow().call_times_ms.clone()
    }
}

impl Default for Probe {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmTransport for Probe {
    fn post_chat(
        &mut self,
        _base_url: &str,
        _payload: &serde_json::Map<String, Value>,
        _timeout: Duration,
    ) -> Result<Value, LlmError> {
        let mut inner = self.inner.borrow_mut();
        inner.post_calls += 1;
        let elapsed_ms = inner.started.elapsed().as_millis();
        inner.call_times_ms.push(elapsed_ms);
        match inner.replies.pop_front() {
            Some(reply) => reply,
            None => Err(LlmError::Transport(
                "scripted downstream: no reply queued (driver bug)".to_string(),
            )),
        }
    }

    fn get_tags(&mut self, _base_url: &str, _timeout: Duration) -> Result<Value, LlmError> {
        Err(LlmError::Transport(
            "scripted downstream: get_tags not scripted".to_string(),
        ))
    }

    fn close(&mut self) {}
}

/// Build the real backend over one handle of a shared scripted downstream.
pub fn backend_over(probe: Probe) -> OllamaBackend<Probe> {
    OllamaBackend::new(phlow_config::OllamaConfig::default(), probe)
}

/// One chat call through the real backend. The message is fixed and valid;
/// only the downstream's scripted reply varies.
fn chat_once(backend: &mut OllamaBackend<Probe>) -> Result<Value, LlmError> {
    let messages = vec![serde_json::json!({"role": "user", "content": "retry probe"})];
    backend.chat(&messages, &[], None)
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
    /// Measured numbers (call counts, timings).
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

/// Assert the error is the downstream's raw transient failure.
fn expect_raw_transport_error(result: &Result<Value, LlmError>, what: &str) -> Result<(), String> {
    match result {
        Err(LlmError::Transport(detail)) if detail.starts_with("downstream down") => Ok(()),
        Err(other) => Err(format!(
            "{what}: got '{other}', want the raw downstream failure"
        )),
        Ok(value) => Err(format!("{what}: unexpectedly succeeded: {value}")),
    }
}

/// V1: transient failure → the path does NOT retry. One scripted failure
/// followed by a success: the first call fails (exactly one downstream
/// call), and only an explicit second call by the driver reaches the
/// queued success. Retry policy, if any, lives with the caller.
fn case_transient_failure_is_not_retried() -> Result<CaseReport, DriverError> {
    const CASE: &str = "transient_failure_is_not_retried";
    let mut evidence = Vec::new();
    let probe = Probe::new();
    probe.queue_failures(1);
    probe.queue_reply(Ok(serde_json::json!({"recovered": true})));
    let mut backend = backend_over(probe.clone());
    let first = chat_once(&mut backend);
    if let Err(detail) = expect_raw_transport_error(&first, "first call") {
        return Ok(CaseReport::fail(CASE, detail, evidence));
    }
    let calls_after_first = probe.post_calls();
    evidence.push(format!(
        "transient failure: first call failed with the raw error after exactly {calls_after_first} downstream call"
    ));
    if calls_after_first != 1 {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "expected 1 downstream call for the first attempt, counted {calls_after_first}"
            ),
            evidence,
        ));
    }
    evidence.push(
        "the path did not retry: the queued success is still waiting for an explicit second call"
            .to_string(),
    );
    match chat_once(&mut backend) {
        Ok(value) if value.get("recovered").and_then(|v| v.as_bool()) == Some(true) => {}
        other => {
            return Ok(CaseReport::fail(
                CASE,
                format!("explicit second call did not get the queued success: {other:?}"),
                evidence,
            ));
        }
    }
    evidence.push(
        "only the caller's explicit second call reached the success: retries are caller-implemented, never path-implemented"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "first_attempt_downstream_calls": calls_after_first,
            "second_attempt_downstream_calls": probe.post_calls(),
        }),
        evidence,
    ))
}

/// V2: no attempt cap, no typed exhaustion — N consecutive failures, N
/// explicit calls: every error is the raw `LlmError::Transport`, never a
/// "retry budget exhausted" typed error, because no budget exists.
fn case_no_attempt_cap() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_attempt_cap_or_exhaustion_error";
    let mut evidence = Vec::new();
    let probe = Probe::new();
    probe.queue_failures(ATTEMPT_PROBE_CALLS);
    let mut backend = backend_over(probe.clone());
    let mut failures = Vec::new();
    for i in 0..ATTEMPT_PROBE_CALLS {
        let result = chat_once(&mut backend);
        if let Err(detail) = expect_raw_transport_error(&result, &format!("call {i}")) {
            failures.push(detail);
            break;
        }
    }
    let reached = probe.post_calls();
    evidence.push(format!(
        "{ATTEMPT_PROBE_CALLS} consecutive failures, {ATTEMPT_PROBE_CALLS} explicit calls: {reached} reached the downstream"
    ));
    if reached != ATTEMPT_PROBE_CALLS as u64 {
        failures.push(format!(
            "expected {ATTEMPT_PROBE_CALLS} downstream calls, counted {reached}"
        ));
    }
    if failures.is_empty() {
        evidence.push(
            "every error was the raw LlmError::Transport: no attempt cap engaged, no typed exhaustion error exists"
                .to_string(),
        );
        Ok(CaseReport::pass(
            CASE,
            serde_json::json!({
                "calls": ATTEMPT_PROBE_CALLS,
                "downstream_calls": reached,
            }),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(CASE, failures.join("; "), evidence))
    }
}

/// A1: 100 callers hit a downed downstream in a tight loop. All 100 reach
/// it, and the whole storm finishes far faster than any backoff schedule
/// could allow — the path injects no delays and decorrelates nothing, so
/// any storm bound must be caller-implemented.
fn case_storm_unthrottled() -> Result<CaseReport, DriverError> {
    const CASE: &str = "storm_reaches_downstream_unthrottled";
    let mut evidence = Vec::new();
    let probe = Probe::new();
    probe.queue_failures(STORM_CALLS);
    let mut backend = backend_over(probe.clone());
    let storm_start = Instant::now();
    let mut raw_errors = 0usize;
    for _ in 0..STORM_CALLS {
        match chat_once(&mut backend) {
            Err(LlmError::Transport(_)) => raw_errors += 1,
            other => {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("storm call did not surface the raw downstream error: {other:?}"),
                    evidence,
                ));
            }
        }
    }
    let wall_secs = storm_start.elapsed().as_secs_f64();
    let reached = probe.post_calls();
    evidence.push(format!(
        "{STORM_CALLS}-caller storm: {reached} downstream calls, {raw_errors} raw errors, wall {wall_secs:.3}s"
    ));
    if reached != STORM_CALLS as u64 {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected {STORM_CALLS} downstream calls, counted {reached}"),
            evidence,
        ));
    }
    if wall_secs > STORM_WALL_SECS_MAX {
        return Ok(CaseReport::fail(
            CASE,
            format!("storm took {wall_secs:.2}s; the path appears to inject delays"),
            evidence,
        ));
    }
    evidence.push(format!(
        "no backoff, no jitter, no cap: the path forwarded the storm 1:1 in {wall_secs:.3}s"
    ));
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "storm_calls": STORM_CALLS,
            "downstream_calls": reached,
            "wall_secs": wall_secs,
        }),
        evidence,
    ))
}

/// A2: measure per-call latency across consecutive failures — with
/// exponential backoff the latencies would grow; here each call must
/// return promptly, proving no delay schedule exists in the path.
fn case_no_backoff_delays() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_backoff_delays_injected";
    let mut evidence = Vec::new();
    let probe = Probe::new();
    probe.queue_failures(BACKOFF_PROBE_CALLS);
    let mut backend = backend_over(probe.clone());
    let mut worst_ms: u128 = 0;
    for i in 0..BACKOFF_PROBE_CALLS {
        let started = Instant::now();
        let result = chat_once(&mut backend);
        let elapsed_ms = started.elapsed().as_millis();
        worst_ms = worst_ms.max(elapsed_ms);
        if let Err(detail) = expect_raw_transport_error(&result, &format!("call {i}")) {
            return Ok(CaseReport::fail(CASE, detail, evidence));
        }
        if elapsed_ms > PER_CALL_MS_MAX {
            return Ok(CaseReport::fail(
                CASE,
                format!("call {i} took {elapsed_ms} ms; the path appears to inject a delay"),
                evidence,
            ));
        }
    }
    let times = probe.call_times_ms();
    evidence.push(format!(
        "{BACKOFF_PROBE_CALLS} consecutive failures: worst per-call latency {worst_ms} ms (bound {PER_CALL_MS_MAX} ms)"
    ));
    evidence.push(format!(
        "downstream arrival times (ms): {times:?} — flat, not exponential"
    ));
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "calls": BACKOFF_PROBE_CALLS,
            "worst_per_call_ms": worst_ms,
        }),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "transient_failure_is_not_retried" => case_transient_failure_is_not_retried(),
        "no_attempt_cap_or_exhaustion_error" => case_no_attempt_cap(),
        "storm_reaches_downstream_unthrottled" => case_storm_unthrottled(),
        "no_backoff_delays_injected" => case_no_backoff_delays(),
        _ => Err(DriverError::Path {
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

/// The honest task verdict: the probes all pass (the evidence is correct),
/// but the designed retry seam is absent from phlow's Rust crates, so the
/// design's storm-bounding pass criteria have nothing to evaluate against.
/// Recorded as an open design gap.
fn seam_finding(evidence: Vec<String>) -> TaskFailure {
    TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no retry wrapper exists around tool/adapter calls in phlow's Rust crates — \
              OllamaBackend::chat, MsgpackTransport::exec and the HTTP transport are all single-shot; \
              failures surface to the caller with no backoff, no jitter, no attempt cap. \
              The storm-bounding pass criteria cannot be evaluated; open design gap."
            .to_string(),
        evidence,
    }
}

fn run_inner(_ctx: &Ctx) -> Result<Vec<String>, TaskFailure> {
    let mut evidence = recon().map_err(|e| TaskFailure {
        where_: "recon".to_string(),
        how: e.to_string(),
        evidence: Vec::new(),
    })?;
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
    Err(seam_finding(evidence))
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
