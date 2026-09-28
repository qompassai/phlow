//! task-28: idempotency keys (rust).
//!
//! Drives phlow's real submission-dedup point: the experiment
//! `Scheduler`'s admission ledger
//! (`crates/phlow-experiment/src/control_plane.rs`). `Scheduler::admit`
//! keys nodes by caller-supplied [`NodeId`][1] — the idempotency key — and
//! refuses a second admission of the same key with
//! [`ExperimentError::DuplicateNode`][2], so the side effect (admission)
//! runs exactly once per key. Four cases: the same key twice admits once;
//! different keys admit twice; the same key with a *different* payload is
//! rejected with `DuplicateNode` naming the key while the stored node is
//! left untouched (never silently executed or overwritten); and there is
//! no key expiry — an old key re-submitted is rejected forever,
//! documented, with the unbounded key-set growth noted as the cost.
//!
//! Semantic deltas from the design's ideal, documented not hidden: the
//! second submission gets `DuplicateNode` (an error), not the first
//! result handle — a client must catch-and-refetch; and there is no TTL,
//! so keys never become "new" again. The safety properties the design's
//! pass criteria name (exactly-once side effects per key; the conflict
//! case returns an explicit error naming the key) hold, so the verdict is
//! `pass`.
//!
//! [1]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-experiment/src/control_plane.rs`, `NodeId::new`)
//! [2]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-experiment/src/error.rs`, `ExperimentError::DuplicateNode`)

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{
    CapabilitySet, ExperimentError, ExperimentId, NodeId, NodeParams, RunId, Scheduler,
    SchedulerLimits, SchedulerNode, WorkerRole,
};
use std::fmt;

/// Task id.
pub const ID: &str = "task-28";
/// Human-readable name.
pub const NAME: &str = "idempotency keys";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "same_key_twice_admits_once",
    "different_keys_admit_twice",
    "same_key_different_payload_rejected",
    "no_key_expiry_documented",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-28 driver itself (not of the code under test).
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
                write!(f, "task-28: cannot build fixture {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

// ---------------------------------------------------------------------------
// Fixtures: the real Scheduler, real node constructors
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

/// Build a real node whose idempotency key is `key` and whose payload is
/// `input_digest`.
fn make_node(
    key: &str,
    input_digest: &str,
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
        input_digest: input_digest.to_string(),
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
    /// Measured numbers (admission counts).
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

/// Assert the error is the explicit key-conflict rejection naming `key`.
fn expect_duplicate_node(result: &Result<(), ExperimentError>, key: &str) -> Result<(), String> {
    match result {
        Err(ExperimentError::DuplicateNode { id }) if id == key => Ok(()),
        Err(other) => Err(format!(
            "second submission of key '{key}' gave '{other}', want DuplicateNode naming the key"
        )),
        Ok(()) => Err(format!(
            "second submission of key '{key}' was admitted: the key did not dedup"
        )),
    }
}

/// V1: the same idempotency key submitted twice → one admission. The
/// second submission is rejected with `DuplicateNode` naming the key —
/// the side effect ran exactly once.
fn case_same_key_twice() -> Result<CaseReport, DriverError> {
    const CASE: &str = "same_key_twice_admits_once";
    let mut evidence = Vec::new();
    let (run, exp) = make_ids("idem1")?;
    let caps = make_capabilities()?;
    let mut sched = make_scheduler()?;
    sched
        .admit(make_node("idem-1", "payload-1", &run, &exp, &caps)?)
        .map_err(|e| fixture_error("first admit", e))?;
    let second = sched.admit(make_node("idem-1", "payload-1", &run, &exp, &caps)?);
    if let Err(detail) = expect_duplicate_node(&second, "idem-1") {
        return Ok(CaseReport::fail(CASE, detail, evidence));
    }
    let stored = sched
        .node(&NodeId::new("idem-1").map_err(|e| fixture_error("node id", e))?)
        .ok_or_else(|| fixture_error("lookup", "admitted node vanished"))?;
    evidence.push(
        "key 'idem-1' submitted twice: first admitted, second rejected with DuplicateNode { id: \"idem-1\" }"
            .to_string(),
    );
    evidence.push(format!(
        "ledger holds the key exactly once (stored input digest: '{}')",
        stored.input_digest()
    ));
    evidence.push(
        "semantic delta (documented): the second submission gets an error, not the first result handle — a client must catch DuplicateNode and re-fetch"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"admissions_ok": 1, "duplicates_rejected": 1}),
        evidence,
    ))
}

/// V2: different keys → two admissions. The dedup is per-key, not global.
fn case_different_keys() -> Result<CaseReport, DriverError> {
    const CASE: &str = "different_keys_admit_twice";
    let mut evidence = Vec::new();
    let (run, exp) = make_ids("idem2")?;
    let caps = make_capabilities()?;
    let mut sched = make_scheduler()?;
    for key in ["idem-a", "idem-b"] {
        sched
            .admit(make_node(key, "payload", &run, &exp, &caps)?)
            .map_err(|e| fixture_error("admit", e))?;
    }
    evidence.push("keys 'idem-a' and 'idem-b': both admitted — dedup is per-key".to_string());
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"admissions_ok": 2, "duplicates_rejected": 0}),
        evidence,
    ))
}

/// A1: the same key with a *different* payload → rejected as a conflict,
/// not silently executed. The stored node's payload is untouched: the
/// second submission changed nothing.
fn case_conflicting_payload() -> Result<CaseReport, DriverError> {
    const CASE: &str = "same_key_different_payload_rejected";
    let mut evidence = Vec::new();
    let (run, exp) = make_ids("idem3")?;
    let caps = make_capabilities()?;
    let mut sched = make_scheduler()?;
    sched
        .admit(make_node("idem-x", "payload-1", &run, &exp, &caps)?)
        .map_err(|e| fixture_error("first admit", e))?;
    let second = sched.admit(make_node("idem-x", "payload-2", &run, &exp, &caps)?);
    if let Err(detail) = expect_duplicate_node(&second, "idem-x") {
        return Ok(CaseReport::fail(CASE, detail, evidence));
    }
    let stored = sched
        .node(&NodeId::new("idem-x").map_err(|e| fixture_error("node id", e))?)
        .ok_or_else(|| fixture_error("lookup", "admitted node vanished"))?;
    if stored.input_digest() != "payload-1" {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "conflicting re-submission overwrote the stored payload: '{}'",
                stored.input_digest()
            ),
            evidence,
        ));
    }
    evidence.push(
        "key 'idem-x' re-submitted with a different payload: rejected with DuplicateNode { id: \"idem-x\" } — an explicit conflict error naming the key"
            .to_string(),
    );
    evidence.push(
        "stored payload unchanged ('payload-1'): the conflicting submission was not silently executed or applied"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"admissions_ok": 1, "conflicts_rejected": 1}),
        evidence,
    ))
}

/// A2: key expiry — there is none. An old key re-submitted is rejected
/// forever (never treated as new). Documented honestly, with the cost:
/// the key set grows without bound (no eviction).
fn case_no_key_expiry() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_key_expiry_documented";
    let mut evidence = Vec::new();
    let (run, exp) = make_ids("idem4")?;
    let caps = make_capabilities()?;
    let mut sched = make_scheduler()?;
    sched
        .admit(make_node("idem-old", "payload-1", &run, &exp, &caps)?)
        .map_err(|e| fixture_error("first admit", e))?;
    // No clock is advanced here because there is no TTL to advance past:
    // the ledger has no expiry field at all (verified by source scan in
    // the doc's recon note). Re-submission must still be rejected.
    let second = sched.admit(make_node("idem-old", "payload-1", &run, &exp, &caps)?);
    if let Err(detail) = expect_duplicate_node(&second, "idem-old") {
        return Ok(CaseReport::fail(CASE, detail, evidence));
    }
    evidence.push(
        "key 'idem-old' re-submitted: rejected with DuplicateNode — keys never expire, never become 'new' again"
            .to_string(),
    );
    evidence.push(
        "documented: the design's TTL scenario becomes 'no TTL exists' — conservative (no double-execution ever), with the cost that the admitted key set grows without bound (no eviction)"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"expiry_mechanism": "none", "resubmissions_rejected": 1}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "same_key_twice_admits_once" => case_same_key_twice(),
        "different_keys_admit_twice" => case_different_keys(),
        "same_key_different_payload_rejected" => case_conflicting_payload(),
        "no_key_expiry_documented" => case_no_key_expiry(),
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
        "recon: Scheduler::admit keys nodes by caller-supplied NodeId and refuses re-admission with ExperimentError::DuplicateNode (crates/phlow-experiment/src/control_plane.rs, error.rs)".to_string(),
        "recon: the admission ledger is the submission-dedup point — no separate idempotency-key layer exists, and none is needed for the dedup property".to_string(),
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
        "verdict: pass — side-effecting admission runs exactly once per key; conflicts return an explicit key-naming error"
            .to_string(),
    );
    evidence.push(
        "documented deltas: (1) second submission gets DuplicateNode, not the first result handle — catch-and-refetch is the client's job; (2) no key TTL/expiry — permanent rejection, unbounded key-set growth"
            .to_string(),
    );
    Ok(evidence)
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
