//! task-34: adversarial concurrent merge (rust).
//!
//! Drives phlow's real run-metadata write path with two scripted writers
//! racing on one record, looking for the design's merge seam: a merge
//! function that combines concurrent writes losslessly, surfacing
//! unmergeable conflicts as explicit conflicts instead of silently
//! resolving them.
//!
//! Honest result: the seam is ABSENT. No merge function exists anywhere
//! in the workspace (source scan: no `fn merge` outside this driver).
//! Concurrent writes to the same [`EvaluationRecord`][1] field are plain
//! last-writer-wins: both writes are acknowledged, the second silently
//! overwrites the first, and no conflict is ever surfaced — the design's
//! adversarial case (same-field concurrent write) demonstrably loses an
//! acknowledged write. Disjoint fields do not clobber each other and
//! concurrent appends to the same event list both survive (Vec push), but
//! those are single-writer-at-a-time appends, not a merge.
//!
//! Four cases, all against the real record (no mocks): two validation,
//! two adversarial. Each case documents the real behavior; the task-level
//! verdict is `fail` at `"seam"` because the design's pass criteria (no
//! acknowledged write is lost in any merge; unmergeable conflicts become
//! explicit conflicts) are violated by the demonstrated silent loss.
//!
//! [1]: https://github.com/qompassai/phlow (local path
//! `crates/phlow-experiment/src/record.rs`, `EvaluationRecord`)

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{EvaluationRecord, RecordParams};
use std::fmt;

/// Task id.
pub const ID: &str = "task-34";
/// Human-readable name.
pub const NAME: &str = "adversarial concurrent merge";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "disjoint_fields_merge_cleanly",
    "concurrent_appends_both_survive",
    "same_field_write_is_silent_last_writer_wins",
    "no_conflict_is_ever_surfaced",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-34 driver itself (not of the code under test).
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
                write!(f, "task-34: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-34: cannot probe {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

// ---------------------------------------------------------------------------
// Fixtures: the real record; two scripted writers
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

/// Read one string field back out of the record's own JSON. There are no
/// field getters; the serialized form is the observable state.
fn json_field(record: &EvaluationRecord, field: &str) -> Result<Option<String>, DriverError> {
    let json = record
        .to_json()
        .map_err(|e| probe_error("record json", e))?;
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| probe_error("record json parse", e))?;
    Ok(value
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}

/// Read one string-array field back out of the record's own JSON.
fn json_array_field(record: &EvaluationRecord, field: &str) -> Result<Vec<String>, DriverError> {
    let json = record
        .to_json()
        .map_err(|e| probe_error("record json", e))?;
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| probe_error("record json parse", e))?;
    value
        .get(field)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .ok_or_else(|| probe_error(field, "field is not a string array"))
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

/// V1: the two writers touch disjoint fields — writer A sets the
/// candidate revision, writer B adds a security result. Neither clobbers
/// the other: the "merge" of disjoint fields is trivially clean because
/// there is no merge at all, just independent setters.
fn case_disjoint_fields() -> Result<CaseReport, DriverError> {
    const CASE: &str = "disjoint_fields_merge_cleanly";
    let mut evidence = Vec::new();
    let mut record = make_record("merge1")?;
    // Writer A.
    record
        .set_candidate_revision("rev-a")
        .map_err(|e| fixture_error("writer A revision", e))?;
    // Writer B: a disjoint field.
    record
        .add_security_result("sec-ok")
        .map_err(|e| fixture_error("writer B security result", e))?;
    let revision = json_field(&record, "candidate_revision")?;
    let security = json_array_field(&record, "security_results")?;
    if revision.as_deref() != Some("rev-a") || security != ["sec-ok".to_string()] {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "disjoint writes clobbered each other: revision={revision:?}, security={security:?}"
            ),
            evidence,
        ));
    }
    evidence.push(
        "writer A set_candidate_revision('rev-a') -> Ok; writer B add_security_result('sec-ok') -> Ok; both present in the record"
            .to_string(),
    );
    evidence.push(
        "disjoint fields do not clobber: independent setters, no merge function involved"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"writes_acknowledged": 2, "writes_surviving": 2}),
        evidence,
    ))
}

/// V2: both writers append to the same event list — both appends survive
/// (Vec push). This is the design's adversarial append case, and it holds
/// — but it holds because appends never conflict, not because a merge
/// combined them.
fn case_concurrent_appends() -> Result<CaseReport, DriverError> {
    const CASE: &str = "concurrent_appends_both_survive";
    let mut evidence = Vec::new();
    let mut record = make_record("merge2")?;
    // Writer A appends; writer B appends to the same list.
    record
        .record_event("evt-a")
        .map_err(|e| fixture_error("writer A event", e))?;
    record
        .record_event("evt-b")
        .map_err(|e| fixture_error("writer B event", e))?;
    let events = json_array_field(&record, "events")?;
    if events != ["evt-a".to_string(), "evt-b".to_string()] {
        return Ok(CaseReport::fail(
            CASE,
            format!("concurrent appends lost data: events={events:?}"),
            evidence,
        ));
    }
    evidence.push(
        "writer A record_event('evt-a') -> Ok; writer B record_event('evt-b') -> Ok; both events present in order"
            .to_string(),
    );
    evidence.push(
        "both appends survive — but this is Vec push with no conflicting write, not a merge: there is still no merge function combining anything"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"appends_acknowledged": 2, "appends_surviving": 2}),
        evidence,
    ))
}

/// A1: both writers write the SAME field. Both writes are acknowledged
/// (`Ok`), the second silently overwrites the first — writer A's
/// acknowledged write is lost with no conflict surfaced. This is the
/// design's adversarial case, and it breaks: no merge, just
/// last-writer-wins data loss.
fn case_same_field_lww() -> Result<CaseReport, DriverError> {
    const CASE: &str = "same_field_write_is_silent_last_writer_wins";
    let mut evidence = Vec::new();
    let mut record = make_record("merge3")?;
    // Writer A: acknowledged.
    record
        .set_candidate_revision("rev-a")
        .map_err(|e| fixture_error("writer A revision", e))?;
    // Writer B: acknowledged — same field, concurrent intent.
    record
        .set_candidate_revision("rev-b")
        .map_err(|e| fixture_error("writer B revision", e))?;
    let revision = json_field(&record, "candidate_revision")?;
    if revision.as_deref() != Some("rev-b") {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected the LWW demonstration ('rev-b' wins), got {revision:?}"),
            evidence,
        ));
    }
    evidence.push(
        "writer A set_candidate_revision('rev-a') -> Ok (acknowledged); writer B set_candidate_revision('rev-b') -> Ok (acknowledged)"
            .to_string(),
    );
    evidence.push(
        "final candidate_revision is 'rev-b': writer A's ACKNOWLEDGED write was silently lost — no merge combined the intents, no conflict was surfaced"
            .to_string(),
    );
    evidence.push(
        "this is the design's adversarial case and it fails the design's pass criterion: an acknowledged write was lost in the 'merge'"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"writes_acknowledged": 2, "writes_surviving": 1, "conflicts_surfaced": 0}),
        evidence,
    ))
}

/// A2: after the silent loss, the record offers no conflict primitive at
/// all — no version, no conflict marker, no error. The serialized record
/// is indistinguishable from a world where writer A never wrote. A
/// delete-vs-update conflict (the design's second adversarial case) has
/// no seam either: the record exposes no field-deletion API, so the
/// conflict cannot even be expressed, let alone surfaced explicitly.
fn case_no_conflict_surfaced() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_conflict_is_ever_surfaced";
    let mut evidence = Vec::new();
    let mut record = make_record("merge4")?;
    record
        .set_candidate_revision("rev-a")
        .map_err(|e| fixture_error("writer A revision", e))?;
    record
        .set_candidate_revision("rev-b")
        .map_err(|e| fixture_error("writer B revision", e))?;
    let json = record
        .to_json()
        .map_err(|e| probe_error("record json", e))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| probe_error("record json parse", e))?;
    let mentions_conflict = json.to_lowercase().contains("conflict");
    let has_version_field = parsed.get("version").is_some() || parsed.get("revision").is_some();
    if mentions_conflict || has_version_field {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "expected no conflict/version surface, found mentions_conflict={mentions_conflict} has_version_field={has_version_field}"
            ),
            evidence,
        ));
    }
    evidence.push(
        "after the silent overwrite, the serialized record contains no conflict marker and no version field — it is indistinguishable from a world where writer A never wrote"
            .to_string(),
    );
    evidence.push(
        "the record exposes no field-deletion API either (source scan): the design's delete-vs-update conflict scenario has no seam — the conflict cannot be expressed, let alone surfaced as an explicit conflict"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"conflict_markers": 0, "version_fields": 0, "delete_apis": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "disjoint_fields_merge_cleanly" => case_disjoint_fields(),
        "concurrent_appends_both_survive" => case_concurrent_appends(),
        "same_field_write_is_silent_last_writer_wins" => case_same_field_lww(),
        "no_conflict_is_ever_surfaced" => case_no_conflict_surfaced(),
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
        "recon: no merge function exists anywhere in the workspace (source scan: no `fn merge` outside this driver); concurrent writes to one EvaluationRecord go through independent LWW setters".to_string(),
        "recon: EvaluationRecord exposes no field-deletion API and no version field — the delete-vs-update conflict scenario has no seam".to_string(),
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
        how: "seam absent: no merge path exists — concurrent same-field writes are silent last-writer-wins (2 acknowledged, 1 surviving, 0 conflicts surfaced; the lost write is unrecoverable and indistinguishable from never having happened). The design's pass criteria (no acknowledged write lost; unmergeable conflicts become explicit conflicts) are violated by the demonstrated data loss.".to_string(),
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
