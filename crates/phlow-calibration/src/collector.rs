//! Collect calibration inputs from the design's three sources.
//!
//! - **Gauntlet traces** are the one tuple-bearing source today:
//!   the JSONL the gauntlet's `TraceRecorder` writes (see
//!   `phlow_gauntlet::trace`) parses into [`CalibrationTuple`]s and
//!   per-task outcomes. Parsing is fail-closed, mirroring the
//!   estate's other trust-adjacent JSONL readers: a malformed line
//!   rejects the whole file, because silent skips would hide a
//!   corrupt corpus.
//! - **The feedback store** (`feedback.db`) is read through
//!   [`FeedbackStore`]'s own API, and only when the file already
//!   exists — collecting never creates it. Its rows are
//!   session-level (a rating and an outcome for a whole session,
//!   with no decision point), so they are collected as weak-label
//!   signals and yield no tuples; see the report for the standing
//!   explanation.
//! - **The consumed-approvals store** is the promotion replay
//!   store (`{"id", "expires_ms"}` lines, as `phlow-experiment`
//!   defines it). Its ids prove approvals were consumed; they
//!   carry no state, question, prediction, or decision, so they
//!   also yield no tuples. Approval *decision* history — the
//!   design's 2.0-weight source — is persisted nowhere in phlow
//!   today; until it is, this source reports its state honestly
//!   and contributes nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use phlow_gauntlet::trace::{
    DecisionTrace, GroundTruth as TraceGroundTruth, TRACE_LINE_BYTES_MAX, is_held_out,
};
use phlow_self_improve::feedback::LOW_RATED_LIMIT_MAX;
use phlow_self_improve::{FeedbackEntry, FeedbackStore};
use phlow_system1::{Answer, AnswerBatch, Question, QuestionBatch};
use serde_json::Value;

use crate::CalibrationError;
use crate::tuple::{CalibrationTuple, GroundTruth, TupleSource};

/// Largest trace file read in one collection (64 MiB). The real
/// corpora are a few hundred KiB; the bound exists so a stray file
/// cannot be loaded whole into memory.
pub const TRACE_FILE_BYTES_MAX: u64 = 64 * 1024 * 1024;

/// Largest consumed-approvals store read in one collection
/// (1 MiB), mirroring `phlow-experiment`'s own bound for the file.
pub const APPROVAL_FILE_BYTES_MAX: u64 = 1_048_576;

/// Longest approval id accepted, mirroring `phlow-experiment`'s
/// record bound for the store.
pub const APPROVAL_ID_BYTES_MAX: usize = 64;

/// Whether a store file was there to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceState {
    /// The file exists and was read.
    Present,
    /// The file does not exist; nothing was created in its place.
    Absent,
}

/// One gauntlet task's realized outcome, from the trace file's
/// `task_outcome` records (joinable to tuples by task id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskOutcome {
    /// Whether the task's traces sit in the held-out reserve.
    pub held_out: bool,
    /// Whether the task passed.
    pub passed: bool,
    /// When the outcome was recorded, epoch milliseconds.
    pub recorded_at_unix_ms: u64,
}

/// The gauntlet source, collected.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GauntletCollection {
    /// Outcome records that repeated a task id (the last wins).
    pub duplicate_outcomes: usize,
    /// `task_outcome` records read, duplicates included.
    pub outcome_records: usize,
    /// Per-task outcomes by task id.
    pub outcomes: BTreeMap<String, TaskOutcome>,
    /// `decision_trace` records read.
    pub trace_records: usize,
    /// The tuples the traces parsed into, in file order.
    pub tuples: Vec<CalibrationTuple>,
}

/// The feedback source, collected.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackCollection {
    /// Mean rating over the store, when it holds any rows.
    pub average_rating: Option<f64>,
    /// Rows read, newest first, capped at the store's own listing
    /// bound ([`LOW_RATED_LIMIT_MAX`]).
    pub entries: Vec<FeedbackEntry>,
    /// Whether the store file was there to read.
    pub state: SourceState,
}

/// The consumed-approvals source, collected.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalCollection {
    /// Consumed approval ids, in file order.
    pub consumed_ids: Vec<String>,
    /// Whether the store file was there to read.
    pub state: SourceState,
}

/// All three sources, collected.
#[derive(Debug, Clone, PartialEq)]
pub struct Collection {
    /// The consumed-approvals store.
    pub approvals: ApprovalCollection,
    /// The feedback store.
    pub feedback: FeedbackCollection,
    /// The gauntlet trace file, when one was configured.
    pub gauntlet: Option<GauntletCollection>,
}

impl Collection {
    /// Every tuple collected (today only the gauntlet source bears
    /// tuples; the other two report signals — see the module docs).
    pub fn tuples(&self) -> &[CalibrationTuple] {
        match &self.gauntlet {
            Some(gauntlet) => &gauntlet.tuples,
            None => &[],
        }
    }
}

/// Where to collect from. Paths are explicit: this crate never
/// guesses where Matt's data lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionPaths {
    /// The consumed-approvals store (conventionally
    /// `$XDG_DATA_HOME/phlow/consumed-approvals.jsonl`).
    pub approval_store: PathBuf,
    /// The feedback store database (`feedback.db`).
    pub feedback_db: PathBuf,
    /// The gauntlet decision-trace JSONL, when a corpus is in use.
    pub trace_file: Option<PathBuf>,
}

/// Collect all three sources. A missing store file is a state,
/// not an error; a malformed record anywhere is an error.
pub fn collect_all(paths: &CollectionPaths) -> Result<Collection, CalibrationError> {
    let gauntlet = match &paths.trace_file {
        Some(path) => Some(collect_gauntlet_traces(path)?),
        None => None,
    };
    Ok(Collection {
        approvals: collect_approval_store(&paths.approval_store)?,
        feedback: collect_feedback(&paths.feedback_db)?,
        gauntlet,
    })
}

/// Parse a gauntlet decision-trace JSONL file into tuples and
/// per-task outcomes.
///
/// Fail-closed: file over [`TRACE_FILE_BYTES_MAX`], a line over
/// [`TRACE_LINE_BYTES_MAX`], malformed JSON, an unknown record
/// kind, a record that fails the trace contract's own validation,
/// or a held-out stamp that disagrees with the task-ID split all
/// reject the whole file.
pub fn collect_gauntlet_traces(path: &Path) -> Result<GauntletCollection, CalibrationError> {
    let bytes = read_bounded(path, TRACE_FILE_BYTES_MAX)?;
    let text = String::from_utf8(bytes).map_err(|_| CalibrationError::Validation {
        reason: format!("{} is not valid UTF-8", path.display()),
    })?;
    let mut collection = GauntletCollection::default();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > TRACE_LINE_BYTES_MAX {
            return Err(CalibrationError::LineTooLong {
                path: path.to_path_buf(),
                line: line_no,
                bytes: line.len(),
                max: TRACE_LINE_BYTES_MAX,
            });
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|error| malformed(path, line_no, &error.to_string()))?;
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed(path, line_no, "record has no string 'kind'"))?;
        match kind {
            "decision_trace" => {
                collection
                    .tuples
                    .push(parse_decision_trace(path, line_no, &value)?);
                collection.trace_records += 1;
            }
            "task_outcome" => {
                let (task_id, outcome) = parse_task_outcome(path, line_no, &value)?;
                if collection.outcomes.insert(task_id, outcome).is_some() {
                    collection.duplicate_outcomes += 1;
                }
                collection.outcome_records += 1;
            }
            other => {
                return Err(CalibrationError::UnknownRecordKind {
                    path: path.to_path_buf(),
                    line: line_no,
                    kind: other.to_string(),
                });
            }
        }
    }
    Ok(collection)
}

/// Read the feedback store when it exists. An absent file is
/// [`SourceState::Absent`] and is never created by collecting.
pub fn collect_feedback(db_path: &Path) -> Result<FeedbackCollection, CalibrationError> {
    if !db_path.is_file() {
        return Ok(FeedbackCollection {
            average_rating: None,
            entries: Vec::new(),
            state: SourceState::Absent,
        });
    }
    let store = FeedbackStore::open(db_path).map_err(feedback_error)?;
    let entries = store
        .get_low_rated(i64::MAX, LOW_RATED_LIMIT_MAX)
        .map_err(feedback_error)?;
    let average_rating = if entries.is_empty() {
        None
    } else {
        Some(store.average_rating().map_err(feedback_error)?)
    };
    Ok(FeedbackCollection {
        average_rating,
        entries,
        state: SourceState::Present,
    })
}

/// Read the consumed-approvals store when it exists, under the
/// same fail-closed rules `phlow-experiment` loads it with
/// (size bound, well-formed `{"id", "expires_ms"}` lines, no
/// duplicate ids). The ids are signals only — see the module docs.
pub fn collect_approval_store(path: &Path) -> Result<ApprovalCollection, CalibrationError> {
    if !path.is_file() {
        return Ok(ApprovalCollection {
            consumed_ids: Vec::new(),
            state: SourceState::Absent,
        });
    }
    let bytes = read_bounded(path, APPROVAL_FILE_BYTES_MAX)?;
    let text = String::from_utf8(bytes).map_err(|_| CalibrationError::ApprovalStore {
        path: path.to_path_buf(),
        line: 0,
        reason: "store is not valid UTF-8".to_string(),
    })?;
    let mut consumed_ids: Vec<String> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let store_error = |reason: &str| CalibrationError::ApprovalStore {
            path: path.to_path_buf(),
            line: line_no,
            reason: reason.to_string(),
        };
        let value: Value =
            serde_json::from_str(line).map_err(|_| store_error("store is corrupt"))?;
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= APPROVAL_ID_BYTES_MAX)
            .ok_or_else(|| store_error("store is corrupt"))?;
        if value.get("expires_ms").and_then(Value::as_u64).is_none() {
            return Err(store_error("store is corrupt"));
        }
        if consumed_ids.iter().any(|seen| seen == id) {
            return Err(store_error("store has a duplicate id"));
        }
        consumed_ids.push(id.to_string());
    }
    Ok(ApprovalCollection {
        consumed_ids,
        state: SourceState::Present,
    })
}

/// Read a whole file under a byte bound (checked from metadata
/// before any bytes are loaded).
fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, CalibrationError> {
    let io_error = |error: std::io::Error| CalibrationError::Io {
        path: path.to_path_buf(),
        reason: error.to_string(),
    };
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    if metadata.len() > max {
        return Err(CalibrationError::FileTooLarge {
            path: path.to_path_buf(),
            bytes: metadata.len(),
            max,
        });
    }
    std::fs::read(path).map_err(io_error)
}

/// A feedback-store error, carried as text (the store's error
/// type is its own; the calibration report needs the reason, not
/// the store's type).
fn feedback_error(error: impl std::fmt::Display) -> CalibrationError {
    CalibrationError::FeedbackStore {
        reason: error.to_string(),
    }
}

/// A malformed-line error for `path` at `line`.
fn malformed(path: &Path, line: usize, reason: &str) -> CalibrationError {
    CalibrationError::MalformedLine {
        path: path.to_path_buf(),
        line,
        reason: reason.to_string(),
    }
}

/// A required string field of a trace record.
fn record_string<'a>(
    path: &Path,
    line: usize,
    record: &'a Value,
    field: &str,
) -> Result<&'a str, CalibrationError> {
    record.get(field).and_then(Value::as_str).ok_or_else(|| {
        malformed(
            path,
            line,
            &format!("record field '{field}' is not a string"),
        )
    })
}

/// A required u64 field of a trace record.
fn record_u64(
    path: &Path,
    line: usize,
    record: &Value,
    field: &str,
) -> Result<u64, CalibrationError> {
    record
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| malformed(path, line, &format!("record field '{field}' is not a u64")))
}

/// A required bool field of a trace record.
fn record_bool(
    path: &Path,
    line: usize,
    record: &Value,
    field: &str,
) -> Result<bool, CalibrationError> {
    record
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| malformed(path, line, &format!("record field '{field}' is not a bool")))
}

/// Parse one `decision_trace` record into a tuple, validating it
/// through the trace contract's own [`DecisionTrace::validate`]
/// and then the tuple contract.
fn parse_decision_trace(
    path: &Path,
    line: usize,
    record: &Value,
) -> Result<CalibrationTuple, CalibrationError> {
    let task_id = record_string(path, line, record, "task_id")?.to_string();
    let question_id = record_string(path, line, record, "question_id")?.to_string();
    let state = record_string(path, line, record, "state")?.to_string();
    let question: Question = serde_json::from_value(
        record
            .get("question")
            .cloned()
            .ok_or_else(|| malformed(path, line, "record has no 'question'"))?,
    )
    .map_err(|error| malformed(path, line, &format!("question does not decode: {error}")))?;
    let predicted = parse_predicted(path, line, record, &question_id, &question, &state)?;
    let predicted_dist = parse_distribution(path, line, record)?;
    let trace_truth = parse_ground_truth(path, line, record)?;
    let held_out = record_bool(path, line, record, "held_out")?;
    let decided_at_unix_ms = record_u64(path, line, record, "decided_at_unix_ms")?;
    let trace = DecisionTrace {
        decided_at_unix_ms,
        ground_truth: trace_truth,
        held_out,
        predicted_dist: predicted_dist.clone(),
        predicted,
        question: question.clone(),
        question_id,
        state: state.clone(),
        task_id: task_id.clone(),
    };
    trace
        .validate()
        .map_err(|error| malformed(path, line, &format!("trace fails validation: {error}")))?;
    let tuple = CalibrationTuple {
        state,
        question,
        predicted,
        predicted_dist,
        ground_truth: trace_truth.map(GroundTruth::from_trace),
        source: TupleSource::Gauntlet { task_id },
        decided_at_unix_ms,
        held_out,
    };
    tuple
        .validate()
        .map_err(|error| malformed(path, line, &error.to_string()))?;
    Ok(tuple)
}

/// Decode a record's `predicted` answer through the protocol's
/// real wire boundary ([`AnswerBatch::from_wire`]) against a
/// one-question batch, so kind and range checks are the protocol's
/// own, not a re-implementation.
fn parse_predicted(
    path: &Path,
    line: usize,
    record: &Value,
    question_id: &str,
    question: &Question,
    state: &str,
) -> Result<Answer, CalibrationError> {
    let predicted_value = record
        .get("predicted")
        .cloned()
        .ok_or_else(|| malformed(path, line, "record has no 'predicted'"))?;
    let batch = QuestionBatch {
        state: state.to_string(),
        questions: BTreeMap::from([(question_id.to_string(), question.clone())]),
    };
    let mut body = serde_json::Map::new();
    body.insert(question_id.to_string(), predicted_value);
    let body_bytes = serde_json::to_vec(&Value::Object(body))
        .map_err(|error| malformed(path, line, &error.to_string()))?;
    let answers = AnswerBatch::from_wire(&batch, &body_bytes)
        .map_err(|error| malformed(path, line, &format!("answer does not decode: {error}")))?;
    answers
        .answers
        .get(question_id)
        .copied()
        .ok_or_else(|| malformed(path, line, "answer batch lost the record's answer"))
}

/// Parse a record's `predicted_dist`: `null` stays `None`; an
/// array must be finite f64s (the trace validation then checks it
/// is a genuine distribution for the question).
fn parse_distribution(
    path: &Path,
    line: usize,
    record: &Value,
) -> Result<Option<Vec<f64>>, CalibrationError> {
    match record.get("predicted_dist") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let dist: Vec<f64> = serde_json::from_value(value.clone()).map_err(|error| {
                malformed(
                    path,
                    line,
                    &format!("distribution does not decode: {error}"),
                )
            })?;
            if dist.iter().any(|p| !p.is_finite()) {
                return Err(malformed(path, line, "distribution has a non-finite entry"));
            }
            Ok(Some(dist))
        }
    }
}

/// Parse a record's `ground_truth` (`null` = label pending) into
/// the trace contract's shape.
fn parse_ground_truth(
    path: &Path,
    line: usize,
    record: &Value,
) -> Result<Option<TraceGroundTruth>, CalibrationError> {
    let Some(value) = record.get("ground_truth") else {
        return Err(malformed(path, line, "record has no 'ground_truth'"));
    };
    if value.is_null() {
        return Ok(None);
    }
    let tag = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed(path, line, "ground truth has no string 'type'"))?;
    let truth = match tag {
        "Choice" => {
            let correct_index = value
                .get("correct_index")
                .and_then(Value::as_u64)
                .ok_or_else(|| malformed(path, line, "choice truth has no 'correct_index'"))?;
            let correct_index = usize::try_from(correct_index)
                .map_err(|_| malformed(path, line, "correct_index overflows usize"))?;
            TraceGroundTruth::Choice { correct_index }
        }
        "Noul" => {
            let actually_yes = value
                .get("actually_yes")
                .and_then(Value::as_bool)
                .ok_or_else(|| malformed(path, line, "noul truth has no 'actually_yes'"))?;
            TraceGroundTruth::Noul { actually_yes }
        }
        "Score" => {
            let true_value = value
                .get("true_value")
                .and_then(Value::as_f64)
                .ok_or_else(|| malformed(path, line, "score truth has no 'true_value'"))?;
            TraceGroundTruth::Score { true_value }
        }
        other => {
            return Err(malformed(
                path,
                line,
                &format!("unknown ground truth type '{other}'"),
            ));
        }
    };
    Ok(Some(truth))
}

/// Parse one `task_outcome` record. The held-out stamp must agree
/// with the task-ID split, exactly as the recorder stamps it.
fn parse_task_outcome(
    path: &Path,
    line: usize,
    record: &Value,
) -> Result<(String, TaskOutcome), CalibrationError> {
    let task_id = record_string(path, line, record, "task_id")?.to_string();
    let held_out = record_bool(path, line, record, "held_out")?;
    if held_out != is_held_out(&task_id) {
        return Err(malformed(
            path,
            line,
            "outcome held-out stamp disagrees with the task-ID split",
        ));
    }
    Ok((
        task_id,
        TaskOutcome {
            held_out,
            passed: record_bool(path, line, record, "passed")?,
            recorded_at_unix_ms: record_u64(path, line, record, "recorded_at_unix_ms")?,
        },
    ))
}
