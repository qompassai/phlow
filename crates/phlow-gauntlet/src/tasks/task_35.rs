//! task-35: duplicate delivery dedup (rust).
//!
//! Probes phlow's real delivery paths looking for the design's event-bus
//! seam: an internal bus/dispatcher that delivers events at-least-once,
//! with consumer-side dedup keyed on bus-assigned event ids (durable
//! across consumer restarts).
//!
//! Honest result: the seam is ABSENT. phlow has no internal event bus or
//! dispatcher. The only delivery paths are request/response RPC —
//! [`MsgpackTransport`][1] (msgpack-RPC over the Neovim socket, dials
//! lazily, no subscriptions) and [`ReqwestTransport`][1] (Ollama HTTP) —
//! plus imperative in-process calls. There is no `run.finished` event,
//! no subscriber list, no delivery function, and no dedup keyed on
//! delivery: the keyed-dedup primitives that do exist
//! ([`Scheduler::admit`][2] keyed by caller-supplied [`NodeId`][2],
//! approval replay ids) live at submission/approval time, not at
//! delivery — the design explicitly distinguishes these (task-28,
//! task-11). A duplicate delivery cannot be injected because there is no
//! delivery to duplicate.
//!
//! Four recon cases against the real crates (no mocks): two validation,
//! two adversarial. Each case documents the real API surface; the
//! task-level verdict is `fail` at `"seam"` because the design's pass
//! criteria (a counted side effect increments exactly once per unique
//! event id across restarts) have no bus, no consumer, and no dedup
//! state to assert against.
//!
//! [1]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-runtime/src/transport/`, `MsgpackTransport`,
//! `ReqwestTransport`)
//! [2]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-experiment/src/control_plane.rs`, `Scheduler::admit`)

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{
    CapabilitySet, ExperimentError, ExperimentId, NodeId, NodeParams, RunId, Scheduler,
    SchedulerLimits, SchedulerNode, WorkerRole,
};
use phlow_runtime::transport::MsgpackTransport;
use std::fmt;

/// Task id.
pub const ID: &str = "task-35";
/// Human-readable name.
pub const NAME: &str = "duplicate delivery dedup";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "runtime_transports_are_request_response_only",
    "keyed_dedup_is_submission_side_only",
    "no_event_consumer_to_deliver_to",
    "restart_dedup_state_is_vacuous",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-35 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture could not be built.
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
                write!(f, "task-35: cannot build fixture {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

// ---------------------------------------------------------------------------
// Fixtures: the real transport; the real scheduler (submission-side dedup)
// ---------------------------------------------------------------------------

fn fixture_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Fixture {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

fn make_scheduler() -> Result<Scheduler, DriverError> {
    let limits = SchedulerLimits {
        queue_capacity: 64,
        ..Default::default()
    };
    Scheduler::new(limits).map_err(|e| fixture_error("scheduler", e))
}

fn make_ids(tag: &str) -> Result<(RunId, ExperimentId), DriverError> {
    let run = RunId::new(&format!("run-{tag}")).map_err(|e| fixture_error("run id", e))?;
    let exp =
        ExperimentId::new(&format!("exp-{tag}")).map_err(|e| fixture_error("experiment id", e))?;
    Ok((run, exp))
}

fn make_capabilities() -> Result<CapabilitySet, DriverError> {
    CapabilitySet::new(
        vec!["read".to_string()],
        vec!["workspace".to_string()],
        64,
        65_536,
    )
    .map_err(|e| fixture_error("capabilities", e))
}

fn make_node(
    key: &str,
    run: &RunId,
    exp: &ExperimentId,
    caps: &CapabilitySet,
) -> Result<SchedulerNode, DriverError> {
    let node_id = NodeId::new(key).map_err(|e| fixture_error("node id", e))?;
    SchedulerNode::new(NodeParams {
        run_id: run.clone(),
        experiment_id: exp.clone(),
        baseline_revision: "rev-1".to_string(),
        workspace_snapshot: "snap-1".to_string(),
        node_id,
        parent_node_id: None,
        role: WorkerRole::Implementer,
        capabilities: caps.clone(),
        input_digest: "input-1".to_string(),
        dependency_ids: Vec::new(),
        generation: 0,
        attempt: 0,
        deadline_ms: 300_000,
        cpu_budget_ms: 1_000,
        memory_budget_bytes: 1_048_576,
        output_bytes_max: 4_096,
        tool_calls_remaining: 10,
    })
    .map_err(|e| fixture_error("node", e))
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

/// V1: construct the real `MsgpackTransport` and document the real
/// delivery surface. The transport module exposes exactly two
/// transports — msgpack-RPC (request/response over the Neovim socket)
/// and Ollama HTTP — neither is an event bus: no subscribe, no
/// broadcast, no delivery-dedup. Construction dials lazily, so probing
/// the type needs no peer.
fn case_transports_request_response() -> Result<CaseReport, DriverError> {
    const CASE: &str = "runtime_transports_are_request_response_only";
    let mut evidence = Vec::new();
    // Real construction against the real type. Nothing connects: the
    // worker dials lazily on the first request, so this is side-effect
    // free (no socket, no peer).
    let _transport = MsgpackTransport::new("gauntlet-probe-unused.sock");
    evidence.push(
        "constructed the real phlow_runtime::transport::MsgpackTransport (lazy dial: no connection, no peer, no side effects)"
            .to_string(),
    );
    evidence.push(
        "the transport module's public surface is exactly { MsgpackTransport, ReqwestTransport }: msgpack-RPC request/response over the Neovim socket, and Ollama HTTP — no event bus, no dispatcher, no subscribe/broadcast API"
            .to_string(),
    );
    evidence.push(
        "with no bus, 'the bus delivers the same event twice' cannot be staged: there is no delivery path to duplicate"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"transports": 2, "event_buses": 0, "subscribe_apis": 0}),
        evidence,
    ))
}

/// V2: the keyed dedup that DOES exist lives at submission time and is
/// caller-keyed, not bus-assigned at delivery. Drive it behaviorally:
/// admit a node twice under one `NodeId` — the second submission is
/// rejected with `DuplicateNode`. This is task-28's seam, not a
/// delivery-dedup seam: the key is supplied by the caller at submission,
/// never assigned by a bus at delivery.
fn case_dedup_is_submission_side() -> Result<CaseReport, DriverError> {
    const CASE: &str = "keyed_dedup_is_submission_side_only";
    let mut evidence = Vec::new();
    let (run, exp) = make_ids("dedup1")?;
    let caps = make_capabilities()?;
    let mut sched = make_scheduler()?;
    sched
        .admit(make_node("delivery-1", &run, &exp, &caps)?)
        .map_err(|e| fixture_error("first admit", e))?;
    let second = sched.admit(make_node("delivery-1", &run, &exp, &caps)?);
    match second {
        Err(ExperimentError::DuplicateNode { id }) if id == "delivery-1" => {
            evidence.push(
                "second admission under NodeId 'delivery-1' -> DuplicateNode { id: \"delivery-1\" }: the dedup key is caller-supplied at submission"
                    .to_string(),
            );
        }
        other => {
            return Ok(CaseReport::fail(
                CASE,
                format!("expected DuplicateNode for the re-admitted key, got {other:?}"),
                evidence,
            ));
        }
    }
    evidence.push(
        "this is the submission-side idempotency seam (task-28), not delivery dedup: no bus assigns an event id at delivery, because no bus delivers events"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"submission_keyed_dedups": 1, "delivery_keyed_dedups": 0}),
        evidence,
    ))
}

/// A1: there is no event consumer to deliver to twice. No `run.finished`
/// event exists in the workspace's event vocabulary, and neither
/// transport exposes a subscription API — the design's default scenario
/// (duplicate `run.finished` → side effect runs once) has no consumer
/// whose side effect could be counted.
fn case_no_consumer() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_event_consumer_to_deliver_to";
    let evidence = vec![
        "source scan: no `run.finished` event type exists in phlow-runtime or phlow-experiment; the only 'publish' is Runtime::publish_cycle_report, which assembles a report value in-process — not an event delivery".to_string(),
        "neither MsgpackTransport nor ReqwestTransport exposes subscribe/deliver: without a subscriber list there is no consumer, and without a consumer there is no counted side effect to dedup".to_string(),
        "the design's default scenario cannot be staged: a duplicate cannot be injected into a delivery path that does not exist".to_string(),
    ];
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"event_consumers": 0, "run_finished_events": 0}),
        evidence,
    ))
}

/// A2: the restart scenario is vacuous — dedup state that does not exist
/// cannot be durable or in-memory. The design's adversarial case
/// (duplicate arrives after the consumer restarted; dedup state must be
/// durable) has no state to inspect: there is nothing to persist, and
/// nothing that a restart could lose.
fn case_restart_vacuous() -> Result<CaseReport, DriverError> {
    const CASE: &str = "restart_dedup_state_is_vacuous";
    let evidence = vec![
        "no delivery-dedup state exists in phlow-runtime or phlow-experiment: no seen-event-id set, no consumer offsets, no outbox".to_string(),
        "the durable states that DO exist (admitted NodeIds, consumed approval ids) are submission/approval-side and keyed by caller-supplied ids — restarting around them is task-28/task-11 territory, not delivery dedup".to_string(),
        "the design's restart scenario has no seam: there is no dedup state whose durability could be asserted".to_string(),
    ];
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"delivery_dedup_states": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "runtime_transports_are_request_response_only" => case_transports_request_response(),
        "keyed_dedup_is_submission_side_only" => case_dedup_is_submission_side(),
        "no_event_consumer_to_deliver_to" => case_no_consumer(),
        "restart_dedup_state_is_vacuous" => case_restart_vacuous(),
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
        "recon: phlow has no internal event bus or dispatcher — the delivery surface is phlow_runtime::transport::{MsgpackTransport, ReqwestTransport}: request/response RPC and Ollama HTTP, no subscribe/broadcast (crates/phlow-runtime/src/transport/)".to_string(),
        "recon: the keyed-dedup primitives that exist are submission/approval-side (Scheduler::admit keyed by caller NodeId; approval replay ids) — none is a bus-assigned delivery id (the design distinguishes these: task-28, task-11)".to_string(),
        "recon: no `run.finished` event type and no event consumer exist; the only 'publish' is in-process report assembly (Runtime::publish_cycle_report)".to_string(),
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
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no internal event bus or dispatcher exists — delivery is request/response RPC and HTTP with no subscribe/broadcast, no `run.finished` event, no consumer, and no delivery-keyed dedup state. A duplicate delivery cannot be staged because there is no delivery to duplicate; the design's pass criterion (exactly-once side effects per unique event id across restarts) has no seam to assert against.".to_string(),
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
