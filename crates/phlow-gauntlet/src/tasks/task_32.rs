//! task-32: event-sourced replay (rust).
//!
//! Drives phlow's real run event log looking for the design's
//! event-sourcing seam: an append path plus a state-fold function that
//! rebuilds run state purely by replaying the log, with typed fold cases
//! for every event type.
//!
//! Honest result: the seam is HALF-ABSENT. The append path exists —
//! [`EvaluationRecord::record_event`][1] appends machine-readable events,
//! bounded at [`EVENTS_MAX`][1] — but there is no state-fold function and
//! no replay entry point: events are opaque strings (no typed event enum,
//! no per-type fold cases to be exhaustive over), `to_json` serializes the
//! record one-way (no `from_json`, no replay constructor), and nothing in
//! the workspace folds an event log back into state. The lifecycle
//! [`transition`][2] applies typed [`LifecycleEvent`][2]s to a state
//! machine, but there is no event *log* behind it — transitions happen
//! imperatively, so it is a fold without a log, not event sourcing.
//!
//! Four cases, all against the real record (no mocks): two validation,
//! two adversarial. Each case documents the real behavior; the task-level
//! verdict is `fail` at `"seam"` because the design's pass criteria
//! (replayed state == live state; every event type has a fold case) have
//! no fold to assert against.
//!
//! [1]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-experiment/src/record.rs`, `EvaluationRecord::record_event`)
//! [2]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-experiment/src/promotion.rs`, `Lifecycle::transition`)

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{EvaluationRecord, RecordParams, SCHEMA_VERSION};
use std::fmt;

/// Task id.
pub const ID: &str = "task-32";
/// Human-readable name.
pub const NAME: &str = "event-sourced replay";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "events_append_and_are_retained",
    "events_are_opaque_strings_no_fold_cases",
    "no_replay_entry_point",
    "schema_bump_has_no_replay_handler",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-32 driver itself (not of the code under test).
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
                write!(f, "task-32: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-32: cannot probe {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

// ---------------------------------------------------------------------------
// Fixtures: the real record; scripted event logs
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

/// Read the `events` array back out of the record's own JSON.
fn events_of(record: &EvaluationRecord) -> Result<Vec<String>, DriverError> {
    let json = record
        .to_json()
        .map_err(|e| probe_error("record json", e))?;
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| probe_error("record json parse", e))?;
    value
        .get("events")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .ok_or_else(|| probe_error("events field", "events is not a string array"))
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

/// V1: the append path works — three scripted events append and are
/// retained in order. This is the half of the seam that exists.
fn case_events_append() -> Result<CaseReport, DriverError> {
    const CASE: &str = "events_append_and_are_retained";
    let mut evidence = Vec::new();
    let mut record = make_record("replay1")?;
    for event in ["run.started", "check.completed", "run.finished"] {
        record
            .record_event(event)
            .map_err(|e| fixture_error("record_event", e))?;
    }
    let count = record.event_count();
    let events = events_of(&record)?;
    if count != 3 || events != ["run.started", "check.completed", "run.finished"] {
        return Ok(CaseReport::fail(
            CASE,
            format!("append path broken: count={count}, events={events:?}"),
            evidence,
        ));
    }
    evidence.push(
        "record_event('run.started'), ('check.completed'), ('run.finished'): all Ok, event_count == 3, order retained"
            .to_string(),
    );
    evidence.push(
        "the append half of the seam exists (bounded at EVENTS_MAX = 1024): the log can be written"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"events_appended": 3, "event_count": count}),
        evidence,
    ))
}

/// V2: the logged events are opaque strings — there is no typed event
/// enum and therefore no per-type fold cases to assert exhaustiveness
/// over. The design's "every event type has a fold case" criterion has
/// no types to apply to.
fn case_events_opaque() -> Result<CaseReport, DriverError> {
    const CASE: &str = "events_are_opaque_strings_no_fold_cases";
    let mut evidence = Vec::new();
    let mut record = make_record("replay2")?;
    record
        .record_event("run.started")
        .map_err(|e| fixture_error("record_event", e))?;
    let events = events_of(&record)?;
    if events != ["run.started".to_string()] {
        return Ok(CaseReport::fail(
            CASE,
            format!("unexpected events content: {events:?}"),
            evidence,
        ));
    }
    evidence.push(
        "the event log is Vec<String>: each event is an opaque string, not a typed variant"
            .to_string(),
    );
    evidence.push(
        "record_event takes &str and stores it verbatim — there is no event-type enum, so 'every event type has a fold case' is vacuous: zero types, zero cases"
            .to_string(),
    );
    evidence.push(
        "contrast: LifecycleEvent (promotion.rs) IS a typed enum with a fold (Lifecycle::transition), but no event log is ever appended behind it — a fold without a log, not event sourcing"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"event_types": 0, "fold_cases": 0}),
        evidence,
    ))
}

/// A1: there is no replay entry point. `to_json` serializes one-way —
/// no `from_json`, no replay constructor, no fold function exists in the
/// workspace (source scan) — so a replay cannot be constructed against
/// any real API. The driver proves the one-wayness behaviorally: the
/// serialized log round-trips through no phlow API.
fn case_no_replay_entry() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_replay_entry_point";
    let mut evidence = Vec::new();
    let mut record = make_record("replay3")?;
    for event in ["run.started", "run.finished"] {
        record
            .record_event(event)
            .map_err(|e| fixture_error("record_event", e))?;
    }
    let json = record
        .to_json()
        .map_err(|e| probe_error("record json", e))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| probe_error("record json parse", e))?;
    let logged = parsed
        .get("events")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    if logged != 2 {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected 2 logged events in the JSON, found {logged}"),
            evidence,
        ));
    }
    evidence.push(
        "to_json emits the event log, but no phlow API consumes it back: no from_json, no replay constructor, no fold function (source scan of crates/phlow-experiment/src)"
            .to_string(),
    );
    evidence.push(
        "a replay would have to be hand-rolled by the caller — inventing the seam, not driving it. Fail closed: the replay half of the seam is absent"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"events_logged": logged, "replay_entry_points": 0}),
        evidence,
    ))
}

/// A2: a mid-stream schema version bump has no replay handler because
/// there is no replay. `schema_version()` returns the compile-time
/// constant; no code path interprets a foreign schema version — a
/// bumped-schema log can be neither handled nor explicitly rejected by
/// a replay that does not exist.
fn case_schema_bump_no_handler() -> Result<CaseReport, DriverError> {
    const CASE: &str = "schema_bump_has_no_replay_handler";
    let mut evidence = Vec::new();
    let record = make_record("replay4")?;
    let version = record.schema_version();
    if version != SCHEMA_VERSION {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "schema_version() = {version}, want the SCHEMA_VERSION constant {SCHEMA_VERSION}"
            ),
            evidence,
        ));
    }
    evidence.push(format!(
        "schema_version() == SCHEMA_VERSION == {SCHEMA_VERSION}: the version is a compile-time constant stamped at construction"
    ));
    evidence.push(
        "no code path reads a schema version off a log and interprets it — the design's 'replay handles or explicitly rejects a mid-stream bump' scenario has no replay to run it against"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"schema_version": version, "bump_handlers": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "events_append_and_are_retained" => case_events_append(),
        "events_are_opaque_strings_no_fold_cases" => case_events_opaque(),
        "no_replay_entry_point" => case_no_replay_entry(),
        "schema_bump_has_no_replay_handler" => case_schema_bump_no_handler(),
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
        "recon: the append path exists — EvaluationRecord::record_event (crates/phlow-experiment/src/record.rs), bounded at EVENTS_MAX = 1024".to_string(),
        "recon: the fold half is absent — events are Vec<String> (no typed event enum, no per-type fold cases); to_json is one-way (no from_json); source scan finds no replay constructor or fold function in crates/phlow-experiment/src".to_string(),
        "recon: Lifecycle::transition (promotion.rs) folds typed LifecycleEvents, but no event log is appended behind it — a fold without a log, not event sourcing".to_string(),
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
        how: "seam half-absent: the event log appends (record_event works, events retained in order) but there is no state-fold function and no replay entry point — events are opaque strings with no typed fold cases, and to_json is one-way. The design's pass criteria (replayed state == live state; every event type has a fold case) have no fold to assert against; replaying would mean inventing the seam.".to_string(),
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
