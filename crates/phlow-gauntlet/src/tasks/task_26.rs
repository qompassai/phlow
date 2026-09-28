//! task-26: circuit breaker (rust).
//!
//! Recon task. The design asks for a circuit breaker on phlow's
//! downstream-call path and instructs: "if no breaker exists, the worker
//! builds the test against the call path and documents the gap."
//!
//! The real call path is `OllamaBackend::chat` → `LlmTransport::post_chat`
//! (`crates/phlow-llm/src/transport.rs`): a bare call with no breaker, no
//! admission control, no state machine. The driver scripts
//! failure/success sequences into a `Probe` — which implements the real
//! `LlmTransport` trait, so the mock sits at the downstream boundary and
//! the backend under test is phlow's real code — and demonstrates four
//! honest behaviors: consecutive failures all reach the downstream
//! (nothing fast-fails), recovery is per-call (there is no half-open
//! probe concept), a 50-call failure storm is absorbed 1:1 by the
//! downstream, and each call's outcome is exactly its scripted reply (the
//! path is stateless).
//!
//! The task verdict is then `fail` with `where = "seam"`: the design's
//! breaker pass criteria (fast-fail while open, observable
//! closed→open→half-open→closed transitions, named thresholds) have no
//! breaker to evaluate them against — an open design gap, not a driver
//! error. The integration tests assert the probe evidence is correct.
//!
//! [`OllamaBackend::chat`]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-llm/src/transport.rs`)

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
pub const ID: &str = "task-26";
/// Human-readable name.
pub const NAME: &str = "circuit breaker";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "consecutive_failures_all_reach_downstream",
    "recovery_is_per_call",
    "failure_storm_absorbed_one_to_one",
    "calls_are_stateless",
];

/// Consecutive scripted failures for the flaky-downstream case.
const CONSECUTIVE_FAILURES: usize = 5;
/// Calls in the adversarial failure storm.
const STORM_CALLS: usize = 50;
/// Scripted replies in the statelessness interleave.
const INTERLEAVE_REPLIES: usize = 6;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-26 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A workspace path was missing or unreadable.
    Path {
        /// Which file was wanted.
        what: String,
        /// Path plus I/O detail.
        detail: String,
    },
    /// The recon premise changed: breaker machinery appeared (or the call
    /// path moved).
    ReconChanged {
        /// What changed.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path { what, detail } => write!(f, "task-26: cannot read {what}: {detail}"),
            Self::ReconChanged { detail } => {
                write!(f, "task-26: recon premise changed: {detail}")
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

/// Verify the task's scope premises against the live sources. Fails closed:
/// if phlow ever gains a circuit breaker on this call path, the old
/// absence claims must not silently pass.
fn recon() -> Result<Vec<String>, DriverError> {
    let src = workspace_root()?
        .join("crates")
        .join("phlow-llm")
        .join("src")
        .join("transport.rs");
    let text = read_file(&src, "phlow-llm transport.rs")?;
    // Premise 1: chat calls the transport directly — the downstream-call
    // wrapper the design names.
    if !text.contains("self.transport.post_chat(&self.base_url, &payload, timeout)") {
        return Err(DriverError::ReconChanged {
            detail: "OllamaBackend::chat no longer calls self.transport.post_chat directly; call path changed"
                .into(),
        });
    }
    // Premise 2: no breaker machinery anywhere in the file.
    for needle in [
        "breaker",
        "Breaker",
        "circuit",
        "Circuit",
        "half_open",
        "HalfOpen",
        "fast_fail",
        "fast-fail",
    ] {
        if text.contains(needle) {
            return Err(DriverError::ReconChanged {
                detail: format!(
                    "phlow-llm/src/transport.rs now contains '{needle}'; breaker machinery may exist"
                ),
            });
        }
    }
    Ok(vec![
        "recon: OllamaBackend::chat calls self.transport.post_chat directly — the downstream-call wrapper is a bare call (verified present)".to_string(),
        "recon: no breaker/circuit/half-open/fast-fail machinery in phlow-llm/src/transport.rs (8 needles absent)".to_string(),
        "finding: phlow has no circuit breaker on its downstream-call path — the task-26 breaker seam is absent".to_string(),
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

/// A scripted downstream at the transport boundary: records every
/// `post_chat` call and replays queued replies. The backend under test is
/// real; only the network peer is faked. Shared ownership (`Rc`) lets the
/// driver read the call count after handing the probe to the backend —
/// `OllamaBackend` takes sole ownership of its transport.
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
    let messages = vec![serde_json::json!({"role": "user", "content": "breaker probe"})];
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
    /// Measured numbers (call counts).
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

/// Assert the error is the downstream's raw failure, unmodified by any
/// admission layer: no fast-fail wrapper, no breaker-typed error.
fn expect_raw_transport_error(result: &Result<Value, LlmError>, what: &str) -> Result<(), String> {
    match result {
        Err(LlmError::Transport(detail)) if detail.starts_with("downstream down") => Ok(()),
        Err(other) => Err(format!(
            "{what}: got '{other}', want the raw downstream failure"
        )),
        Ok(value) => Err(format!("{what}: unexpectedly succeeded: {value}")),
    }
}

/// V1: flaky downstream — N consecutive failures. Every one of the N calls
/// must reach the downstream (count them): nothing fast-fails, because
/// there is no breaker to open.
fn case_consecutive_failures() -> Result<CaseReport, DriverError> {
    const CASE: &str = "consecutive_failures_all_reach_downstream";
    let mut evidence = Vec::new();
    let probe = Probe::new();
    probe.queue_failures(CONSECUTIVE_FAILURES);
    let mut backend = backend_over(probe.clone());
    let mut failures = Vec::new();
    for i in 0..CONSECUTIVE_FAILURES {
        let result = chat_once(&mut backend);
        if let Err(detail) = expect_raw_transport_error(&result, &format!("call {i}")) {
            failures.push(detail);
        }
    }
    let reached = probe.post_calls();
    evidence.push(format!(
        "{CONSECUTIVE_FAILURES} consecutive downstream failures: {reached} calls reached the downstream"
    ));
    if reached != CONSECUTIVE_FAILURES as u64 {
        failures.push(format!(
            "expected {CONSECUTIVE_FAILURES} downstream calls, counted {reached}"
        ));
    }
    if failures.is_empty() {
        evidence.push(
            "no call fast-failed: with no breaker, every failure propagates 1:1 to the caller"
                .to_string(),
        );
        Ok(CaseReport::pass(
            CASE,
            serde_json::json!({
                "consecutive_failures": CONSECUTIVE_FAILURES,
                "downstream_calls": reached,
            }),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(CASE, failures.join("; "), evidence))
    }
}

/// V2: recovery is per-call, not half-open — after failures, a scripted
/// success is served immediately by the very next call. There is no probe
/// concept because there is no breaker state to probe.
fn case_recovery_is_per_call() -> Result<CaseReport, DriverError> {
    const CASE: &str = "recovery_is_per_call";
    let mut evidence = Vec::new();
    let probe = Probe::new();
    probe.queue_failures(3);
    probe.queue_reply(Ok(serde_json::json!({"recovered": true})));
    let mut backend = backend_over(probe.clone());
    for i in 0..3 {
        let result = chat_once(&mut backend);
        if expect_raw_transport_error(&result, &format!("failure {i}")).is_err() {
            return Ok(CaseReport::fail(
                CASE,
                "a failure call did not surface the raw downstream error".to_string(),
                evidence,
            ));
        }
    }
    match chat_once(&mut backend) {
        Ok(value) => {
            if value.get("recovered").and_then(|v| v.as_bool()) != Some(true) {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("recovery reply had the wrong shape: {value}"),
                    evidence,
                ));
            }
        }
        Err(e) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("the call after the failures did not succeed: {e}"),
                evidence,
            ));
        }
    }
    let reached = probe.post_calls();
    evidence.push(format!(
        "3 failures then 1 success: the 4th call succeeded immediately ({reached} downstream calls total)"
    ));
    evidence.push(
        "no half-open probe exists: every call is admitted, so 'recovery' is just the next call succeeding"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"calls_before_recovery": 3, "downstream_calls": reached}),
        evidence,
    ))
}

/// A1: the downstream never recovers — a 50-call failure storm. Every call
/// reaches the downstream: with no breaker, the storm is absorbed 1:1.
/// This is the honest cost of the missing breaker, measured not asserted.
fn case_failure_storm() -> Result<CaseReport, DriverError> {
    const CASE: &str = "failure_storm_absorbed_one_to_one";
    let mut evidence = Vec::new();
    let probe = Probe::new();
    probe.queue_failures(STORM_CALLS);
    let mut backend = backend_over(probe.clone());
    let mut failures = Vec::new();
    for i in 0..STORM_CALLS {
        let result = chat_once(&mut backend);
        if let Err(detail) = expect_raw_transport_error(&result, &format!("storm call {i}")) {
            failures.push(detail);
            break;
        }
    }
    let reached = probe.post_calls();
    evidence.push(format!(
        "{STORM_CALLS}-call failure storm: {reached} calls reached the downstream, 0 fast-failed"
    ));
    if reached != STORM_CALLS as u64 {
        failures.push(format!(
            "expected {STORM_CALLS} downstream calls, counted {reached}"
        ));
    }
    if failures.is_empty() {
        evidence.push(
            "a breaker would have capped this at the open threshold; without one the downstream absorbs the full storm"
                .to_string(),
        );
        Ok(CaseReport::pass(
            CASE,
            serde_json::json!({"storm_calls": STORM_CALLS, "downstream_calls": reached}),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(CASE, failures.join("; "), evidence))
    }
}

/// A2: interleave failures and successes — each call's outcome must be
/// exactly its scripted reply, proving the path keeps no cross-call state
/// (no failure counter, no breaker state machine).
fn case_calls_are_stateless() -> Result<CaseReport, DriverError> {
    const CASE: &str = "calls_are_stateless";
    let mut evidence = Vec::new();
    let probe = Probe::new();
    let scripted: [bool; INTERLEAVE_REPLIES] = [false, true, false, false, true, false];
    for (i, ok) in scripted.iter().enumerate() {
        if *ok {
            probe.queue_reply(Ok(serde_json::json!({"seq": i})));
        } else {
            probe.queue_reply(Err(LlmError::Transport(format!("downstream down ({i})"))));
        }
    }
    let mut backend = backend_over(probe.clone());
    let mut failures = Vec::new();
    for (i, want_ok) in scripted.iter().enumerate() {
        match (chat_once(&mut backend), want_ok) {
            (Ok(value), true) => {
                if value.get("seq").and_then(|v| v.as_u64()) != Some(i as u64) {
                    failures.push(format!(
                        "call {i}: success reply had the wrong seq: {value}"
                    ));
                }
            }
            (Err(e), false) => {
                if expect_raw_transport_error(&Err(e), &format!("call {i}")).is_err() {
                    failures.push(format!(
                        "call {i}: failure was not the raw downstream error"
                    ));
                }
            }
            (Ok(value), false) => {
                failures.push(format!("call {i}: expected failure, got success: {value}"))
            }
            (Err(e), true) => failures.push(format!("call {i}: expected success, got: {e}")),
        }
    }
    evidence.push(format!(
        "{} interleaved calls: every outcome matched its scripted reply exactly",
        scripted.len()
    ));
    if failures.is_empty() {
        evidence.push(
            "no cross-call state: a success after failures is served normally, a failure after a success fails normally"
                .to_string(),
        );
        Ok(CaseReport::pass(
            CASE,
            serde_json::json!({
                "interleaved_calls": scripted.len(),
                "downstream_calls": probe.post_calls(),
            }),
            evidence,
        ))
    } else {
        Ok(CaseReport::fail(CASE, failures.join("; "), evidence))
    }
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "consecutive_failures_all_reach_downstream" => case_consecutive_failures(),
        "recovery_is_per_call" => case_recovery_is_per_call(),
        "failure_storm_absorbed_one_to_one" => case_failure_storm(),
        "calls_are_stateless" => case_calls_are_stateless(),
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
/// but the designed breaker seam is absent, so the design's breaker pass
/// criteria have nothing to evaluate against. Recorded as an open design
/// gap.
fn seam_finding(evidence: Vec<String>) -> TaskFailure {
    TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: phlow has no circuit breaker on its downstream-call path — \
              OllamaBackend::chat calls LlmTransport::post_chat directly (crates/phlow-llm/src/transport.rs); \
              no admission control, no fast-fail, no closed/open/half-open state machine. \
              The breaker's pass criteria cannot be evaluated; open design gap."
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
