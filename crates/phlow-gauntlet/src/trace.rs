//! Decision-trace capture for the calibration harness.
//!
//! Design of record: `calibration-harness-design-2026-10-04.md`. The
//! harness measures calibration from tuples of (state, question,
//! predicted answer, full predicted distribution, ground truth). This
//! module is the gauntlet side of that contract: while a run executes,
//! every scored System 1 decision point appends one [`DecisionTrace`]
//! per question to an append-only JSONL file, plus one outcome record
//! per task so realized outcomes stay joinable by task id.
//!
//! # Contract
//!
//! - Capture is off unless a recorder is [`install`]ed (the `gauntlet`
//!   binary does so for `--trace-file`, or for
//!   `<report-dir>/decision-traces.jsonl` when `--report-dir` is
//!   given). Uninstalled, every entry point here is a no-op and task
//!   behavior is unchanged.
//! - Distributions are never fabricated. A Noul answer's scalar
//!   probability *is* its complete Bernoulli distribution and is
//!   recorded as `[1 - p, p]` in [P(no), P(yes)] order — the same
//!   construction [`RichAnswer::validate`] defines. Choice and Score
//!   answers from scalar backends carry no distribution; the trace
//!   records `predicted_dist: null` rather than inventing one (the
//!   `System1Decider::decide_rich` contract forbids expanding scalar
//!   confidence into a pseudo-distribution).
//! - Ground truth is recorded as `null` (label pending, a state the
//!   design defines) unless a caller supplies a label. The gauntlet's
//!   scenario checks are protocol-fidelity checks whose expected
//!   values equal the scripted predictions by construction; harvesting
//!   them as labels would manufacture perfect-calibration tuples.
//! - Fail closed: a validation or write failure is returned to the
//!   caller (the task funnels surface it as the task's own error), so
//!   a capture run never reports green on a corpus it failed to write.
//! - Bounds: at most [`TRACES_PER_SESSION_MAX`] traces per recorder
//!   session and [`TRACE_LINE_BYTES_MAX`] bytes per record line.

use std::collections::HashSet;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use phlow_system1::{Answer, AnswerBatch, Question, QuestionBatch, RichAnswer, STATE_BYTES_MAX};
use serde_json::{Map, Value};

/// Maximum bytes of one serialized trace or outcome record line.
pub const TRACE_LINE_BYTES_MAX: usize = 64 * 1024;
/// Maximum traces one recorder session appends. A runaway capture is a
/// bug, not a corpus; the session fails closed instead of filling disk.
pub const TRACES_PER_SESSION_MAX: usize = 100_000;

/// Held-out buckets: one bucket in [`HELD_OUT_BUCKETS`] is the
/// validation reserve, i.e. 20% of task ids, per the design.
const HELD_OUT_BUCKETS: u64 = 5;

/// True when `task_id` belongs to the held-out validation reserve.
///
/// Deterministic (FNV-1a 64 over the id bytes) so the split is stable
/// across runs and machines, and enforced by task ID as the design
/// requires: the same id is always on the same side of the split.
pub fn is_held_out(task_id: &str) -> bool {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in task_id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash.is_multiple_of(HELD_OUT_BUCKETS)
}

/// The realized correct answer for one question, when a label exists.
///
/// Mirrors the design's `GroundTruth`; the kind must match the
/// question's kind or the trace is rejected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GroundTruth {
    /// The index of the correct option of a Choice question.
    Choice {
        /// Index into the question's options.
        correct_index: usize,
    },
    /// Whether a Noul question's true answer is yes.
    Noul {
        /// True when the correct answer is yes.
        actually_yes: bool,
    },
    /// The true value of a Score question, on 0.0..=1.0.
    Score {
        /// The correct score.
        true_value: f64,
    },
}

/// One scored decision point, in the design's `CalibrationTuple` shape.
///
/// The tuple's `source` is carried as `task_id`
/// (`TupleSource::Gauntlet { task_id }`); `question_id` identifies the
/// question inside the batch the decision came from.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionTrace {
    /// When the decision was made, epoch milliseconds (the design's
    /// `decided_at`, stored without a chrono dependency).
    pub decided_at_unix_ms: u64,
    /// The realized label, or `None` while the label is pending.
    pub ground_truth: Option<GroundTruth>,
    /// Whether `task_id` is in the held-out validation reserve.
    pub held_out: bool,
    /// The full predicted distribution in option order, or `None` when
    /// the backend exposed only a scalar answer.
    pub predicted_dist: Option<Vec<f64>>,
    /// What the model predicted.
    pub predicted: Answer,
    /// The question that was asked.
    pub question: Question,
    /// Id of the question inside its batch.
    pub question_id: String,
    /// The state string the decision was made against.
    pub state: String,
    /// The gauntlet task that made the decision.
    pub task_id: String,
}

impl DecisionTrace {
    /// Build a trace for `task_id`, stamping the held-out side and the
    /// current time. Validation is separate: call
    /// [`DecisionTrace::validate`] (the recorder always does).
    pub fn new(
        task_id: String,
        question_id: String,
        state: String,
        question: Question,
        predicted: Answer,
        predicted_dist: Option<Vec<f64>>,
        ground_truth: Option<GroundTruth>,
    ) -> Self {
        DecisionTrace {
            decided_at_unix_ms: now_unix_ms(),
            ground_truth,
            held_out: is_held_out(&task_id),
            predicted_dist,
            predicted,
            question,
            question_id,
            state,
            task_id,
        }
    }

    /// Serialize as one JSONL record line (keys alphabetical), after
    /// [`DecisionTrace::validate`] passes.
    pub fn to_json_line(&self) -> Result<String, TraceError> {
        self.validate()?;
        let mut map = Map::new();
        map.insert(
            "decided_at_unix_ms".to_string(),
            Value::from(self.decided_at_unix_ms),
        );
        map.insert(
            "ground_truth".to_string(),
            ground_truth_json(self.ground_truth),
        );
        map.insert("held_out".to_string(), Value::from(self.held_out));
        map.insert("kind".to_string(), Value::from("decision_trace"));
        map.insert("predicted".to_string(), answer_json(self.predicted)?);
        map.insert(
            "predicted_dist".to_string(),
            match &self.predicted_dist {
                None => Value::Null,
                Some(dist) => Value::Array(dist.iter().map(|p| Value::from(*p)).collect()),
            },
        );
        map.insert(
            "question".to_string(),
            serde_json::to_value(&self.question).map_err(|error| TraceError::Serialize {
                reason: error.to_string(),
            })?,
        );
        map.insert(
            "question_id".to_string(),
            Value::from(self.question_id.as_str()),
        );
        map.insert("state".to_string(), Value::from(self.state.as_str()));
        map.insert("task_id".to_string(), Value::from(self.task_id.as_str()));
        let line = Value::Object(map).to_string();
        if line.len() > TRACE_LINE_BYTES_MAX {
            return Err(TraceError::LineTooLong {
                bytes: line.len(),
                max: TRACE_LINE_BYTES_MAX,
            });
        }
        Ok(line)
    }

    /// Check the trace against the calibration contract: non-empty
    /// ids, a state within the protocol bound, an answer of the
    /// question's kind with in-range scalars, a ground truth of the
    /// question's kind when present, and — when a distribution is
    /// present — a genuine one, checked by the real
    /// [`RichAnswer::validate`] (right length, finite in-range
    /// entries, sums to 1.0 within tolerance).
    pub fn validate(&self) -> Result<(), TraceError> {
        let field = |field: &'static str, reason: &str| TraceError::FieldInvalid {
            field,
            reason: reason.to_string(),
        };
        if self.task_id.is_empty() {
            return Err(field("task_id", "task id is empty"));
        }
        if self.question_id.is_empty() {
            return Err(field("question_id", "question id is empty"));
        }
        if self.state.len() > STATE_BYTES_MAX {
            return Err(field("state", "state exceeds STATE_BYTES_MAX"));
        }
        let kind_matches = matches!(
            (&self.question, self.predicted),
            (Question::Choice { .. }, Answer::Choice { .. })
                | (Question::Noul { .. }, Answer::Noul { .. })
                | (Question::Score { .. }, Answer::Score { .. })
        );
        if !kind_matches {
            return Err(TraceError::KindMismatch {
                question_id: self.question_id.clone(),
            });
        }
        let confidence = self.predicted.confidence();
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(field(
                "predicted",
                "answer confidence is not a finite number in 0..=1",
            ));
        }
        if let Answer::Score { value, .. } = self.predicted
            && (!value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(field(
                "predicted",
                "score value is not a finite number in 0..=1",
            ));
        }
        if let Some(truth) = self.ground_truth {
            let truth_matches = matches!(
                (&self.question, truth),
                (Question::Choice { .. }, GroundTruth::Choice { .. })
                    | (Question::Noul { .. }, GroundTruth::Noul { .. })
                    | (Question::Score { .. }, GroundTruth::Score { .. })
            );
            if !truth_matches {
                return Err(TraceError::KindMismatch {
                    question_id: self.question_id.clone(),
                });
            }
            if let GroundTruth::Score { true_value } = truth
                && (!true_value.is_finite() || !(0.0..=1.0).contains(&true_value))
            {
                return Err(field(
                    "ground_truth",
                    "true score is not a finite number in 0..=1",
                ));
            }
        }
        if let Some(dist) = &self.predicted_dist {
            if dist.is_empty() {
                return Err(TraceError::DistributionEmpty {
                    question_id: self.question_id.clone(),
                });
            }
            let rich = RichAnswer {
                answer: self.predicted,
                distribution: dist.clone(),
            };
            rich.validate(&self.question)
                .map_err(|error| TraceError::DistributionInvalid {
                    question_id: self.question_id.clone(),
                    reason: error.to_string(),
                })?;
        }
        Ok(())
    }
}

/// Why trace capture failed. Every variant is fail-closed: the caller
/// surfaces it, and no partial or unvalidated record is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceError {
    /// A recorder is already installed in this process.
    AlreadyInstalled,
    /// A present distribution is empty.
    DistributionEmpty {
        /// The question the distribution belonged to.
        question_id: String,
    },
    /// A present distribution is not a genuine distribution.
    DistributionInvalid {
        /// The question the distribution belonged to.
        question_id: String,
        /// Why the distribution was rejected.
        reason: String,
    },
    /// The same task id was begun twice in one recorder session.
    DuplicateTask {
        /// The repeated task id.
        task_id: String,
    },
    /// A trace field violates the contract.
    FieldInvalid {
        /// Which field.
        field: &'static str,
        /// What is wrong with it.
        reason: String,
    },
    /// The trace file could not be written.
    Io {
        /// The I/O failure, as text.
        reason: String,
    },
    /// Answer or ground-truth kind does not match the question kind.
    KindMismatch {
        /// The question whose kinds disagreed.
        question_id: String,
    },
    /// A serialized record exceeds [`TRACE_LINE_BYTES_MAX`].
    LineTooLong {
        /// The record's length in bytes.
        bytes: usize,
        /// The limit it exceeded.
        max: usize,
    },
    /// A record was attempted with no task open.
    NoTaskOpen,
    /// A record could not be serialized.
    Serialize {
        /// The serialization failure, as text.
        reason: String,
    },
    /// The session reached [`TRACES_PER_SESSION_MAX`].
    SessionFull {
        /// The session cap.
        max: usize,
    },
    /// A record or close named a task other than the open one.
    TaskMismatch {
        /// The task currently open.
        open: String,
        /// The task the caller named.
        given: String,
    },
    /// A task was begun while another task is still open.
    TaskOpen {
        /// The task still open.
        task_id: String,
    },
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TraceError::AlreadyInstalled => {
                write!(f, "decision trace recorder is already installed")
            }
            TraceError::DistributionEmpty { question_id } => {
                write!(f, "trace for '{question_id}': distribution is empty")
            }
            TraceError::DistributionInvalid {
                question_id,
                reason,
            } => write!(
                f,
                "trace for '{question_id}': distribution invalid: {reason}"
            ),
            TraceError::DuplicateTask { task_id } => {
                write!(f, "task '{task_id}' was already recorded this session")
            }
            TraceError::FieldInvalid { field, reason } => {
                write!(f, "trace field '{field}': {reason}")
            }
            TraceError::Io { reason } => write!(f, "decision trace write failed: {reason}"),
            TraceError::KindMismatch { question_id } => write!(
                f,
                "trace for '{question_id}': answer kind does not match question kind"
            ),
            TraceError::LineTooLong { bytes, max } => {
                write!(f, "trace record is {bytes} bytes, over the {max} limit")
            }
            TraceError::NoTaskOpen => write!(f, "no task is open on the trace recorder"),
            TraceError::Serialize { reason } => {
                write!(f, "trace record could not be serialized: {reason}")
            }
            TraceError::SessionFull { max } => {
                write!(f, "trace session reached the {max} trace cap")
            }
            TraceError::TaskMismatch { open, given } => {
                write!(f, "trace task mismatch: '{open}' is open, got '{given}'")
            }
            TraceError::TaskOpen { task_id } => {
                write!(f, "trace task '{task_id}' is still open")
            }
        }
    }
}

impl std::error::Error for TraceError {}

impl From<std::io::Error> for TraceError {
    fn from(error: std::io::Error) -> Self {
        TraceError::Io {
            reason: error.to_string(),
        }
    }
}

/// Append-only JSONL writer for one capture session.
///
/// Owns the trace file. One task may be open at a time; traces are
/// attributed to the open task, and [`TraceRecorder::end_task`]
/// appends the task's outcome record. Every record is flushed as it
/// is written, so a crashed run keeps the traces already recorded.
#[derive(Debug)]
pub struct TraceRecorder {
    max_traces: usize,
    open_task: Option<String>,
    outcomes_written: usize,
    path: PathBuf,
    seen_tasks: HashSet<String>,
    traces_written: usize,
    writer: BufWriter<File>,
}

impl TraceRecorder {
    /// Open `path` for appending (creating it and its parent
    /// directory when needed) with the standard session cap.
    pub fn open(path: &Path) -> Result<Self, TraceError> {
        Self::open_with_max(path, TRACES_PER_SESSION_MAX)
    }

    /// Open with an explicit session cap (the cap is a parameter so
    /// tests can exercise the limit without writing a full session).
    fn open_with_max(path: &Path, max_traces: usize) -> Result<Self, TraceError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(TraceRecorder {
            max_traces,
            open_task: None,
            outcomes_written: 0,
            path: path.to_path_buf(),
            seen_tasks: HashSet::new(),
            traces_written: 0,
            writer: BufWriter::new(file),
        })
    }

    /// Begin attributing traces to `task_id`. Fails when another task
    /// is open or this id was already recorded this session.
    pub fn begin_task(&mut self, task_id: &str) -> Result<(), TraceError> {
        if let Some(open) = &self.open_task {
            return Err(TraceError::TaskOpen {
                task_id: open.clone(),
            });
        }
        if !self.seen_tasks.insert(task_id.to_string()) {
            return Err(TraceError::DuplicateTask {
                task_id: task_id.to_string(),
            });
        }
        self.open_task = Some(task_id.to_string());
        Ok(())
    }

    /// Append the outcome record for the open task and close it.
    pub fn end_task(&mut self, task_id: &str, passed: bool) -> Result<(), TraceError> {
        let open = self.open_task.take().ok_or(TraceError::NoTaskOpen)?;
        if open != task_id {
            self.open_task = Some(open.clone());
            return Err(TraceError::TaskMismatch {
                open,
                given: task_id.to_string(),
            });
        }
        let mut map = Map::new();
        map.insert("held_out".to_string(), Value::from(is_held_out(task_id)));
        map.insert("kind".to_string(), Value::from("task_outcome"));
        map.insert("passed".to_string(), Value::from(passed));
        map.insert(
            "recorded_at_unix_ms".to_string(),
            Value::from(now_unix_ms()),
        );
        map.insert("task_id".to_string(), Value::from(task_id));
        let line = Value::Object(map).to_string();
        self.write_line(&line)?;
        self.outcomes_written += 1;
        Ok(())
    }

    /// Validate and append one trace for the open task.
    pub fn record(&mut self, trace: &DecisionTrace) -> Result<(), TraceError> {
        let open = self.open_task.as_deref().ok_or(TraceError::NoTaskOpen)?;
        if open != trace.task_id {
            return Err(TraceError::TaskMismatch {
                open: open.to_string(),
                given: trace.task_id.clone(),
            });
        }
        if self.traces_written >= self.max_traces {
            return Err(TraceError::SessionFull {
                max: self.max_traces,
            });
        }
        let line = trace.to_json_line()?;
        self.write_line(&line)?;
        self.traces_written += 1;
        Ok(())
    }

    /// Record one trace per question in a scored batch: the batch the
    /// decider answered and the answers it returned, already validated
    /// by the caller. Noul answers carry their complete Bernoulli
    /// distribution; other kinds record `predicted_dist: null`
    /// (scalar backend — no distribution exists to record).
    pub fn record_decision(
        &mut self,
        batch: &QuestionBatch,
        answers: &AnswerBatch,
    ) -> Result<(), TraceError> {
        let task_id = self.open_task.clone().ok_or(TraceError::NoTaskOpen)?;
        // Build and validate the whole batch before writing any of
        // it: a batch lands completely or not at all, so a rejected
        // batch can never leave partial traces behind.
        let mut traces = Vec::with_capacity(batch.questions.len());
        for (question_id, question) in &batch.questions {
            let Some(answer) = answers.answers.get(question_id) else {
                return Err(TraceError::FieldInvalid {
                    field: "answers",
                    reason: format!("no answer for question '{question_id}'"),
                });
            };
            let predicted_dist = match *answer {
                Answer::Noul { probability, .. } => Some(vec![1.0 - probability, probability]),
                Answer::Choice { .. } | Answer::Score { .. } => None,
            };
            let trace = DecisionTrace::new(
                task_id.clone(),
                question_id.clone(),
                batch.state.clone(),
                question.clone(),
                *answer,
                predicted_dist,
                None,
            );
            trace.validate()?;
            traces.push(trace);
        }
        for trace in &traces {
            self.record(trace)?;
        }
        Ok(())
    }

    /// How many outcome records this session appended.
    pub fn outcomes_written(&self) -> usize {
        self.outcomes_written
    }

    /// The file this recorder appends to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many traces this session appended.
    pub fn traces_written(&self) -> usize {
        self.traces_written
    }

    fn write_line(&mut self, line: &str) -> Result<(), TraceError> {
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }
}

/// The process-wide recorder, installed at most once. Task dispatch
/// (`run_task`) brackets tasks with [`begin_task`]/[`end_task`]; the
/// decision funnels call [`record_decision`].
static RECORDER: OnceLock<Mutex<TraceRecorder>> = OnceLock::new();

/// Install the process-wide recorder on `path`. A second install is
/// an error, never a silent swap.
pub fn install(path: &Path) -> Result<(), TraceError> {
    let recorder = TraceRecorder::open(path)?;
    RECORDER
        .set(Mutex::new(recorder))
        .map_err(|_| TraceError::AlreadyInstalled)
}

/// True when a recorder is installed (capture is on).
pub fn is_installed() -> bool {
    RECORDER.get().is_some()
}

/// Begin a task on the installed recorder; a no-op when capture is off.
pub fn begin_task(task_id: &str) -> Result<(), TraceError> {
    with_recorder(|recorder| recorder.begin_task(task_id))
}

/// End a task on the installed recorder; a no-op when capture is off.
pub fn end_task(task_id: &str, passed: bool) -> Result<(), TraceError> {
    with_recorder(|recorder| recorder.end_task(task_id, passed))
}

/// Record a scored batch on the installed recorder; a no-op when
/// capture is off. Errors when capture is on but no task is open:
/// an unattributable trace must not enter the corpus silently.
pub fn record_decision(batch: &QuestionBatch, answers: &AnswerBatch) -> Result<(), TraceError> {
    with_recorder(|recorder| recorder.record_decision(batch, answers))
}

/// Traces written by the installed recorder (0 when capture is off).
pub fn traces_written() -> usize {
    RECORDER.get().map_or(0, |recorder| {
        recorder
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .traces_written()
    })
}

fn with_recorder(
    f: impl FnOnce(&mut TraceRecorder) -> Result<(), TraceError>,
) -> Result<(), TraceError> {
    match RECORDER.get() {
        None => Ok(()),
        Some(recorder) => f(&mut recorder.lock().unwrap_or_else(PoisonError::into_inner)),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn answer_json(answer: Answer) -> Result<Value, TraceError> {
    let serialize = |error: String| TraceError::Serialize { reason: error };
    let mut map = Map::new();
    match answer {
        Answer::Choice {
            selected,
            probability,
        } => {
            map.insert("probability".to_string(), Value::from(probability));
            let selected = u64::try_from(selected)
                .map_err(|_| serialize("selected index overflows u64".to_string()))?;
            map.insert("selected".to_string(), Value::from(selected));
        }
        Answer::Noul { yes, probability } => {
            map.insert("probability".to_string(), Value::from(probability));
            map.insert("yes".to_string(), Value::from(yes));
        }
        Answer::Score { value, confidence } => {
            map.insert("confidence".to_string(), Value::from(confidence));
            map.insert("value".to_string(), Value::from(value));
        }
    }
    Ok(Value::Object(map))
}

fn ground_truth_json(truth: Option<GroundTruth>) -> Value {
    let Some(truth) = truth else {
        return Value::Null;
    };
    let mut map = Map::new();
    match truth {
        GroundTruth::Choice { correct_index } => {
            map.insert(
                "correct_index".to_string(),
                Value::from(u64::try_from(correct_index).unwrap_or(u64::MAX)),
            );
            map.insert("type".to_string(), Value::from("Choice"));
        }
        GroundTruth::Noul { actually_yes } => {
            map.insert("actually_yes".to_string(), Value::from(actually_yes));
            map.insert("type".to_string(), Value::from("Noul"));
        }
        GroundTruth::Score { true_value } => {
            map.insert("true_value".to_string(), Value::from(true_value));
            map.insert("type".to_string(), Value::from("Score"));
        }
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temp_trace_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phlow-gauntlet-trace-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("decision-traces.jsonl")
    }

    fn noul_question() -> Question {
        Question::Noul {
            instructions: "Is it reversible?".to_string(),
        }
    }

    fn noul_trace(probability: f64, dist: Option<Vec<f64>>) -> DecisionTrace {
        DecisionTrace::new(
            "task-251".to_string(),
            "q".to_string(),
            "state".to_string(),
            noul_question(),
            Answer::Noul {
                yes: probability > 0.5,
                probability,
            },
            dist,
            None,
        )
    }

    // Validation: a Noul trace with its complete Bernoulli
    // distribution validates and serializes with the design's fields.
    #[test]
    fn noul_trace_validates_and_serializes() {
        let trace = noul_trace(0.75, Some(vec![0.25, 0.75]));
        trace.validate().unwrap();
        let line = trace.to_json_line().unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["kind"], "decision_trace");
        assert_eq!(value["task_id"], "task-251");
        assert_eq!(value["question_id"], "q");
        assert_eq!(value["predicted_dist"], serde_json::json!([0.25, 0.75]));
        assert_eq!(value["predicted"]["yes"], true);
        assert!(value["ground_truth"].is_null());
        assert_eq!(value["held_out"], is_held_out("task-251"));
    }

    // Validation: a Choice trace from a scalar backend (no
    // distribution exists) validates with predicted_dist null.
    #[test]
    fn choice_trace_without_distribution_validates() {
        let trace = DecisionTrace::new(
            "task-254".to_string(),
            "q".to_string(),
            "Do not cancel the pending appointment.".to_string(),
            Question::Choice {
                instructions: "Select the requested action.".to_string(),
                options: vec![
                    "keep appointment".to_string(),
                    "cancel appointment".to_string(),
                ],
            },
            Answer::Choice {
                selected: 0,
                probability: 0.99,
            },
            None,
            Some(GroundTruth::Choice { correct_index: 0 }),
        );
        trace.validate().unwrap();
        let line = trace.to_json_line().unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();
        assert!(value["predicted_dist"].is_null());
        assert_eq!(value["ground_truth"]["correct_index"], 0);
    }

    // Validation: the recorder writes one trace per question plus the
    // outcome record, and the file round-trips as JSONL.
    #[test]
    fn recorder_round_trip_writes_traces_and_outcome() {
        let path = temp_trace_path("round-trip");
        let mut recorder = TraceRecorder::open(&path).unwrap();
        recorder.begin_task("task-251").unwrap();
        let mut questions = BTreeMap::new();
        questions.insert(
            "a_choice".to_string(),
            Question::Choice {
                instructions: "Pick one.".to_string(),
                options: vec!["x".to_string(), "y".to_string()],
            },
        );
        questions.insert("b_noul".to_string(), noul_question());
        let batch = QuestionBatch {
            state: "state".to_string(),
            questions,
        };
        let mut answers = BTreeMap::new();
        answers.insert(
            "a_choice".to_string(),
            Answer::Choice {
                selected: 1,
                probability: 0.8,
            },
        );
        answers.insert(
            "b_noul".to_string(),
            Answer::Noul {
                yes: true,
                probability: 0.9,
            },
        );
        recorder
            .record_decision(&batch, &AnswerBatch { answers })
            .unwrap();
        assert_eq!(recorder.traces_written(), 2);
        recorder.end_task("task-251", true).unwrap();
        assert_eq!(recorder.outcomes_written(), 1);
        drop(recorder);
        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = body
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 3);
        let outcome = lines.last().unwrap();
        assert_eq!(outcome["kind"], "task_outcome");
        assert_eq!(outcome["passed"], true);
        let noul = lines
            .iter()
            .find(|line| line["question_id"] == "b_noul")
            .unwrap();
        let dist = noul["predicted_dist"].as_array().unwrap();
        assert_eq!(dist.len(), 2);
        // [1 - p, p] in f64: 1.0 - 0.9 is not exactly 0.1, and the
        // contract tolerance is DISTRIBUTION_SUM_TOLERANCE, not bits.
        assert!((dist[0].as_f64().unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(dist[1].as_f64().unwrap(), 0.9);
        let choice = lines
            .iter()
            .find(|line| line["question_id"] == "a_choice")
            .unwrap();
        assert!(choice["predicted_dist"].is_null());
    }

    // Validation: the held-out split is deterministic and lands near
    // 20% over the system1 wave's id range.
    #[test]
    fn held_out_is_deterministic_and_roughly_twenty_percent() {
        let mut held = 0;
        for index in 251..=318 {
            let id = format!("task-{index}");
            assert_eq!(is_held_out(&id), is_held_out(&id));
            if is_held_out(&id) {
                held += 1;
            }
        }
        assert!(
            (5..=25).contains(&held),
            "held-out count {held} of 68 is not near 20%"
        );
    }

    // Validation: a Score trace with a Score ground truth validates.
    #[test]
    fn score_trace_with_ground_truth_validates() {
        let trace = DecisionTrace::new(
            "task-252".to_string(),
            "q".to_string(),
            "state".to_string(),
            Question::Score {
                instructions: "Rate the risk.".to_string(),
                criteria: vec!["harm".to_string()],
            },
            Answer::Score {
                value: 0.1,
                confidence: 0.95,
            },
            None,
            Some(GroundTruth::Score { true_value: 0.2 }),
        );
        trace.validate().unwrap();
    }

    // Validation: a full risk batch records one trace per question;
    // the four Noul answers carry distributions, the Score does not.
    #[test]
    fn risk_batch_records_five_traces() {
        let path = temp_trace_path("risk-batch");
        let mut recorder = TraceRecorder::open(&path).unwrap();
        recorder.begin_task("task-251").unwrap();
        let mut questions = BTreeMap::new();
        questions.insert(
            "risk".to_string(),
            Question::Score {
                instructions: "Rate the risk.".to_string(),
                criteria: vec!["harm".to_string()],
            },
        );
        for id in ["reversible", "irreversible", "consistent", "forbidden"] {
            questions.insert(id.to_string(), noul_question());
        }
        let batch = QuestionBatch {
            state: "state".to_string(),
            questions,
        };
        let mut answers = BTreeMap::new();
        answers.insert(
            "risk".to_string(),
            Answer::Score {
                value: 0.1,
                confidence: 0.99,
            },
        );
        for (id, yes) in [
            ("reversible", true),
            ("irreversible", false),
            ("consistent", true),
            ("forbidden", false),
        ] {
            answers.insert(
                id.to_string(),
                Answer::Noul {
                    yes,
                    probability: 0.99,
                },
            );
        }
        recorder
            .record_decision(&batch, &AnswerBatch { answers })
            .unwrap();
        assert_eq!(recorder.traces_written(), 5);
        recorder.end_task("task-251", true).unwrap();
        drop(recorder);
        let body = std::fs::read_to_string(&path).unwrap();
        let traces: Vec<Value> = body
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|line: &Value| line["kind"] == "decision_trace")
            .collect();
        assert_eq!(traces.len(), 5);
        let with_dist = traces
            .iter()
            .filter(|line| line["predicted_dist"].is_array())
            .count();
        assert_eq!(with_dist, 4);
    }

    // Validation: a failed task's outcome record says so.
    #[test]
    fn failed_task_outcome_is_recorded() {
        let path = temp_trace_path("failed-outcome");
        let mut recorder = TraceRecorder::open(&path).unwrap();
        recorder.begin_task("task-299").unwrap();
        recorder.end_task("task-299", false).unwrap();
        drop(recorder);
        let body = std::fs::read_to_string(&path).unwrap();
        let outcome: Value = serde_json::from_str(body.lines().next().unwrap()).unwrap();
        assert_eq!(outcome["kind"], "task_outcome");
        assert_eq!(outcome["passed"], false);
        assert_eq!(outcome["task_id"], "task-299");
    }

    // Validation: reopening the corpus appends; a second session's
    // records accumulate behind the first session's, never truncate.
    #[test]
    fn reopening_the_corpus_appends() {
        let path = temp_trace_path("append");
        {
            let mut recorder = TraceRecorder::open(&path).unwrap();
            recorder.begin_task("task-251").unwrap();
            recorder.record(&noul_trace(0.9, None)).unwrap();
            recorder.end_task("task-251", true).unwrap();
        }
        {
            let mut recorder = TraceRecorder::open(&path).unwrap();
            recorder.begin_task("task-252").unwrap();
            recorder.end_task("task-252", true).unwrap();
        }
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 3);
    }

    // Adversarial: a distribution that does not sum to 1 is rejected.
    #[test]
    fn distribution_not_summing_to_one_is_rejected() {
        let trace = noul_trace(0.9, Some(vec![0.5, 0.4]));
        assert!(matches!(
            trace.validate(),
            Err(TraceError::DistributionInvalid { .. })
        ));
    }

    // Adversarial: an empty distribution is rejected by name, not
    // treated as "no distribution".
    #[test]
    fn empty_distribution_is_rejected() {
        let trace = noul_trace(0.9, Some(vec![]));
        assert!(matches!(
            trace.validate(),
            Err(TraceError::DistributionEmpty { .. })
        ));
    }

    // Adversarial: a distribution with the wrong number of entries
    // for the question is rejected.
    #[test]
    fn distribution_length_mismatch_is_rejected() {
        let trace = DecisionTrace::new(
            "task-254".to_string(),
            "q".to_string(),
            "state".to_string(),
            Question::Choice {
                instructions: "Pick one.".to_string(),
                options: vec!["x".to_string(), "y".to_string()],
            },
            Answer::Choice {
                selected: 0,
                probability: 0.9,
            },
            Some(vec![0.2, 0.3, 0.5]),
            None,
        );
        assert!(matches!(
            trace.validate(),
            Err(TraceError::DistributionInvalid { .. })
        ));
    }

    // Adversarial: non-finite distribution entries are rejected.
    #[test]
    fn nonfinite_distribution_is_rejected() {
        let trace = noul_trace(0.9, Some(vec![f64::NAN, 1.0]));
        assert!(matches!(
            trace.validate(),
            Err(TraceError::DistributionInvalid { .. })
        ));
    }

    // Adversarial: an out-of-range scalar answer is rejected even
    // when no distribution is present to catch it.
    #[test]
    fn answer_confidence_out_of_range_is_rejected() {
        let trace = noul_trace(1.5, None);
        assert!(matches!(
            trace.validate(),
            Err(TraceError::FieldInvalid {
                field: "predicted",
                ..
            })
        ));
    }

    // Adversarial: answer and ground-truth kinds that disagree with
    // the question kind are rejected.
    #[test]
    fn kind_mismatch_is_rejected() {
        let wrong_answer = DecisionTrace::new(
            "task-251".to_string(),
            "q".to_string(),
            "state".to_string(),
            noul_question(),
            Answer::Choice {
                selected: 0,
                probability: 0.9,
            },
            None,
            None,
        );
        assert!(matches!(
            wrong_answer.validate(),
            Err(TraceError::KindMismatch { .. })
        ));
        let wrong_truth = DecisionTrace::new(
            "task-254".to_string(),
            "q".to_string(),
            "state".to_string(),
            Question::Choice {
                instructions: "Pick one.".to_string(),
                options: vec!["x".to_string(), "y".to_string()],
            },
            Answer::Choice {
                selected: 0,
                probability: 0.9,
            },
            None,
            Some(GroundTruth::Noul { actually_yes: true }),
        );
        assert!(matches!(
            wrong_truth.validate(),
            Err(TraceError::KindMismatch { .. })
        ));
    }

    // Adversarial: a write to a failing device surfaces as an I/O
    // error from the record call, never a panic or a silent drop.
    #[test]
    fn write_failure_is_an_error() {
        let mut recorder = TraceRecorder::open(Path::new("/dev/full")).unwrap();
        recorder.begin_task("task-251").unwrap();
        let result = recorder.record(&noul_trace(0.9, Some(vec![0.1, 0.9])));
        assert!(matches!(result, Err(TraceError::Io { .. })), "{result:?}");
    }

    // Adversarial: duplicate task ids in one session are rejected, as
    // is beginning a task while another is open.
    #[test]
    fn duplicate_and_overlapping_tasks_are_rejected() {
        let path = temp_trace_path("duplicates");
        let mut recorder = TraceRecorder::open(&path).unwrap();
        recorder.begin_task("task-251").unwrap();
        assert!(matches!(
            recorder.begin_task("task-252"),
            Err(TraceError::TaskOpen { .. })
        ));
        recorder.end_task("task-251", true).unwrap();
        assert!(matches!(
            recorder.begin_task("task-251"),
            Err(TraceError::DuplicateTask { .. })
        ));
    }

    // Adversarial: recording with no task open, or for a task other
    // than the open one, is rejected — traces must be attributable.
    #[test]
    fn record_without_open_task_is_rejected() {
        let path = temp_trace_path("attribution");
        let mut recorder = TraceRecorder::open(&path).unwrap();
        assert!(matches!(
            recorder.record(&noul_trace(0.9, None)),
            Err(TraceError::NoTaskOpen)
        ));
        recorder.begin_task("task-252").unwrap();
        assert!(matches!(
            recorder.record(&noul_trace(0.9, None)),
            Err(TraceError::TaskMismatch { .. })
        ));
    }

    // Adversarial: the session cap stops a runaway capture.
    #[test]
    fn session_cap_is_enforced() {
        let path = temp_trace_path("cap");
        let mut recorder = TraceRecorder::open_with_max(&path, 1).unwrap();
        recorder.begin_task("task-251").unwrap();
        recorder.record(&noul_trace(0.9, None)).unwrap();
        assert!(matches!(
            recorder.record(&noul_trace(0.8, None)),
            Err(TraceError::SessionFull { max: 1 })
        ));
    }

    // Adversarial: an unknown question type tag is rejected at the
    // protocol boundary, never coerced into a known kind.
    #[test]
    fn unknown_question_type_is_rejected() {
        let parsed: Result<Question, _> = serde_json::from_value(serde_json::json!({
            "type": "Bogus",
            "instructions": "x",
        }));
        assert!(parsed.is_err());
    }
}
