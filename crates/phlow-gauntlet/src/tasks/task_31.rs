//! task-31: optimistic concurrency (rust).
//!
//! Drives phlow's real run-metadata write paths looking for the design's
//! optimistic-concurrency seam: a versioned store where a write carries the
//! version it read, the store compares-and-swaps, and a stale write is
//! rejected with a version-mismatch error naming expected vs actual.
//!
//! Honest result: the seam is ABSENT. [`EvaluationRecord`][1] mutations are
//! plain last-writer-wins — the struct carries no per-record version or
//! revision counter (only the constant `schema_version`), so two writers
//! setting the same field silently overwrite each other: the loser's
//! acknowledged write is lost with no rejection and no conflict. The only
//! version-like check anywhere near the write path is
//! [`Scheduler::publish_result`][2]'s generation pin, which compares the
//! caller-supplied generation against the node's *delegation depth* — a
//! value fixed at admission and never bumped by any write — so it is a
//! stale-handle guard, not optimistic concurrency: there is no version 8,
//! no version 9, and no retry-with-fresh-read path.
//!
//! Four cases, all against the real types (no mocks): two validation, two
//! adversarial. Each case documents the real behavior; the task-level
//! verdict is `fail` at `"seam"` because the design's pass criteria (no
//! lost updates; rejections naming expected vs actual version) are not
//! met — last-writer-wins is the documented finding, exactly the outcome
//! the design names as the fallback.
//!
//! [1]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-experiment/src/record.rs`, `EvaluationRecord`)
//! [2]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-experiment/src/control_plane.rs`,
//! `Scheduler::publish_result`)

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{
    CapabilitySet, EvaluationRecord, ExperimentError, ExperimentId, NodeId, NodeParams, NodeState,
    RecordParams, RunId, Scheduler, SchedulerLimits, SchedulerNode, WorkerRole,
};
use std::fmt;

/// Task id.
pub const ID: &str = "task-31";
/// Human-readable name.
pub const NAME: &str = "optimistic concurrency";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "record_mutations_are_last_writer_wins",
    "publish_generation_pin_never_bumps",
    "ignored_generation_rejected_with_expected_vs_got",
    "second_publish_rejected_never_overwritten",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-31 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture could not be built.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// A JSON probe of a record failed.
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
                write!(f, "task-31: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-31: cannot probe {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

// ---------------------------------------------------------------------------
// Fixtures: the real record and the real scheduler
// ---------------------------------------------------------------------------

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

fn make_record(tag: &str) -> Result<EvaluationRecord, DriverError> {
    EvaluationRecord::new(RecordParams {
        experiment_id: format!("exp-{tag}"),
        baseline_revision: "rev-0".to_string(),
        workspace_digest: "ws-1".to_string(),
        operator_config_digest: "op-1".to_string(),
        model_ids: vec!["model-1".to_string()],
        toolchain_versions: vec!["rust-nightly".to_string()],
        limits: vec!["turns=10".to_string()],
        stop_reason: "gauntlet-probe".to_string(),
    })
    .map_err(|e| fixture_error("evaluation record", e))
}

/// Read the `candidate_revision` back out of the record's own JSON.
/// There is no getter; the serialized form is the observable state.
fn candidate_revision_of(record: &EvaluationRecord) -> Result<Option<String>, DriverError> {
    let json = record
        .to_json()
        .map_err(|e| probe_error("record json", e))?;
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| probe_error("record json parse", e))?;
    Ok(value
        .get("candidate_revision")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
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

fn node_generation(sched: &Scheduler, key: &str) -> Result<u64, DriverError> {
    let id = NodeId::new(key).map_err(|e| fixture_error("node id", e))?;
    sched
        .node(&id)
        .map(SchedulerNode::generation)
        .ok_or_else(|| fixture_error("lookup", "admitted node vanished"))
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

/// V1: two writers set the same record field. Both writes are
/// acknowledged (`Ok`), the second silently overwrites the first — the
/// loser's intent is lost with no version check, no rejection, no
/// conflict. This is the design's named fallback: last-writer-wins,
/// documented as the finding.
fn case_record_mutations_lww() -> Result<CaseReport, DriverError> {
    const CASE: &str = "record_mutations_are_last_writer_wins";
    let mut evidence = Vec::new();
    let mut record = make_record("occ1")?;
    // Writer A: acknowledged.
    record
        .set_candidate_revision("rev-a")
        .map_err(|e| fixture_error("writer A revision", e))?;
    // Writer B: acknowledged — no version offered, none checked.
    record
        .set_candidate_revision("rev-b")
        .map_err(|e| fixture_error("writer B revision", e))?;
    let final_rev = candidate_revision_of(&record)?;
    if final_rev.as_deref() != Some("rev-b") {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected silent last-writer-wins ('rev-b'), got {final_rev:?}"),
            evidence,
        ));
    }
    evidence.push(
        "writer A set_candidate_revision('rev-a') -> Ok; writer B set_candidate_revision('rev-b') -> Ok"
            .to_string(),
    );
    evidence.push(
        "final candidate_revision is 'rev-b': writer A's acknowledged write was silently lost — no version offered, none checked"
            .to_string(),
    );
    evidence.push(
        "EvaluationRecord carries no per-record version or revision counter (only the constant schema_version): there is no compare-and-swap write path to drive"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"writes_acknowledged": 2, "writes_surviving": 1, "rejections": 0}),
        evidence,
    ))
}

/// V2: the generation check in `publish_result` pins the node's
/// delegation depth — a value fixed at admission. Publishing does NOT
/// bump it, so there is no version 8 / version 9 and no
/// retry-with-fresh-read path: this is a stale-handle guard, not
/// optimistic concurrency.
fn case_generation_never_bumps() -> Result<CaseReport, DriverError> {
    const CASE: &str = "publish_generation_pin_never_bumps";
    let mut evidence = Vec::new();
    let (run, exp) = make_ids("occ2")?;
    let caps = make_capabilities()?;
    let mut sched = make_scheduler()?;
    let key = "occ-node-1";
    sched
        .admit(make_node(key, &run, &exp, &caps)?)
        .map_err(|e| fixture_error("admit", e))?;
    let before = node_generation(&sched, key)?;
    sched
        .publish_result(
            &NodeId::new(key).map_err(|e| fixture_error("node id", e))?,
            before,
            "digest-1",
            NodeState::Succeeded,
        )
        .map_err(|e| fixture_error("publish", e))?;
    let after = node_generation(&sched, key)?;
    if before != after {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "generation changed across a write ({before} -> {after}); expected the fixed delegation-depth pin"
            ),
            evidence,
        ));
    }
    evidence.push(format!(
        "node generation before publish: {before}; after publish: {after} — the write did not bump any version"
    ));
    evidence.push(
        "publish_result compares the caller's generation against the node's delegation depth (fixed at admission): a stale-handle guard, not a versioned compare-and-swap"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"generation_before": before, "generation_after": after}),
        evidence,
    ))
}

/// A1: a writer that ignores the version field (passes a wrong
/// generation) is rejected with `StaleGeneration` naming expected vs
/// actual — and nothing is applied: the node stays non-terminal and no
/// result is recorded. The closest existing mechanism behaves, but it
/// guards handle staleness, not write versions.
fn case_ignored_generation_rejected() -> Result<CaseReport, DriverError> {
    const CASE: &str = "ignored_generation_rejected_with_expected_vs_got";
    let mut evidence = Vec::new();
    let (run, exp) = make_ids("occ3")?;
    let caps = make_capabilities()?;
    let mut sched = make_scheduler()?;
    let key = "occ-node-2";
    sched
        .admit(make_node(key, &run, &exp, &caps)?)
        .map_err(|e| fixture_error("admit", e))?;
    let node_id = NodeId::new(key).map_err(|e| fixture_error("node id", e))?;
    let result = sched.publish_result(&node_id, 7, "digest-x", NodeState::Succeeded);
    match result {
        Err(ExperimentError::StaleGeneration {
            node,
            expected,
            got,
        }) => {
            evidence.push(format!(
                "publish with generation 7 on a generation-{expected} node -> StaleGeneration {{ node: '{node}', expected: {expected}, got: {got} }}: the rejection names expected vs actual"
            ));
        }
        Err(other) => {
            return Ok(CaseReport::fail(
                CASE,
                format!(
                    "wrong-generation publish gave '{other}', want StaleGeneration naming expected vs got"
                ),
                evidence,
            ));
        }
        Ok(()) => {
            return Ok(CaseReport::fail(
                CASE,
                "wrong-generation publish was applied blind: the version field was ignored"
                    .to_string(),
                evidence,
            ));
        }
    }
    let node = sched
        .node(&node_id)
        .ok_or_else(|| fixture_error("lookup", "admitted node vanished"))?;
    if node.state().is_terminal() || node.result_digest().is_some() {
        return Ok(CaseReport::fail(
            CASE,
            "the rejected publish still mutated the node: a stale write was partially applied"
                .to_string(),
            evidence,
        ));
    }
    evidence.push(
        "node state unchanged (non-terminal) and no result digest recorded: the stale write was rejected, never applied blind"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"stale_rejected": 1, "blind_applications": 0}),
        evidence,
    ))
}

/// A2: a second publish for the same node is rejected with
/// `DuplicateResult` — the first digest stands, the loser's intent is
/// rejected rather than silently applied. At-most-once publication, not
/// optimistic concurrency: there is still no version to retry against.
fn case_second_publish_rejected() -> Result<CaseReport, DriverError> {
    const CASE: &str = "second_publish_rejected_never_overwritten";
    let mut evidence = Vec::new();
    let (run, exp) = make_ids("occ4")?;
    let caps = make_capabilities()?;
    let mut sched = make_scheduler()?;
    let key = "occ-node-3";
    sched
        .admit(make_node(key, &run, &exp, &caps)?)
        .map_err(|e| fixture_error("admit", e))?;
    let node_id = NodeId::new(key).map_err(|e| fixture_error("node id", e))?;
    sched
        .publish_result(&node_id, 0, "digest-first", NodeState::Succeeded)
        .map_err(|e| fixture_error("first publish", e))?;
    match sched.publish_result(&node_id, 0, "digest-second", NodeState::Succeeded) {
        Err(ExperimentError::DuplicateResult { node }) => {
            evidence.push(format!(
                "second publish -> DuplicateResult {{ node: '{node}' }}: rejected, not applied"
            ));
        }
        Err(other) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("second publish gave '{other}', want DuplicateResult"),
                evidence,
            ));
        }
        Ok(()) => {
            return Ok(CaseReport::fail(
                CASE,
                "second publish overwrote the first: at-most-once violated".to_string(),
                evidence,
            ));
        }
    }
    let node = sched
        .node(&node_id)
        .ok_or_else(|| fixture_error("lookup", "admitted node vanished"))?;
    if node.result_digest() != Some("digest-first") {
        return Ok(CaseReport::fail(
            CASE,
            format!("first digest was overwritten: {:?}", node.result_digest()),
            evidence,
        ));
    }
    evidence.push(
        "first digest 'digest-first' intact: the loser's intent was rejected, never silently applied — at-most-once holds, but there is no version to retry a stale write against"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"publishes_ok": 1, "duplicates_rejected": 1}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "record_mutations_are_last_writer_wins" => case_record_mutations_lww(),
        "publish_generation_pin_never_bumps" => case_generation_never_bumps(),
        "ignored_generation_rejected_with_expected_vs_got" => case_ignored_generation_rejected(),
        "second_publish_rejected_never_overwritten" => case_second_publish_rejected(),
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
        "recon: EvaluationRecord (crates/phlow-experiment/src/record.rs) has no per-record version or revision counter — only the constant schema_version; every mutation is an unchecked overwrite".to_string(),
        "recon: Scheduler::publish_result (crates/phlow-experiment/src/control_plane.rs) pins the caller generation against the node's delegation depth (fixed at admission, never bumped by writes) — a stale-handle guard, not a versioned compare-and-swap".to_string(),
        "recon: no VersionMismatch-style error exists anywhere in the workspace (source scan); the closest rejection is StaleGeneration { expected, got }".to_string(),
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
        "finding: last-writer-wins — two acknowledged writes to the same record field silently lose the first (2 acknowledged, 1 surviving, 0 rejections)".to_string(),
    );
    evidence.push(
        "finding: the generation pin never bumps on write, so there is no version 8 / version 9 and no retry-with-fresh-read path".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no versioned compare-and-swap write path exists — record mutations are last-writer-wins (demonstrated silent overwrite of an acknowledged write) and the only version-like check pins an immutable delegation depth. The design's pass criteria (no lost updates; rejections naming expected vs actual version) are not met; LWW is the documented finding.".to_string(),
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
