//! Replay a mock-backend gauntlet corpus through the rich-answer
//! backend, producing phlow's first honestly labeled calibration
//! corpus.
//!
//! # Why this exists
//!
//! The gauntlet's decision-trace corpora (design:
//! `calibration-harness-design-2026-10-04.md`) were captured against
//! the scripted `MockBackend`: the scenario authors scripted both
//! the predictions and the checks, so those traces carry
//! `ground_truth: null` by design — scoring them would manufacture
//! calibration. This binary re-asks every decision point recorded
//! in such a corpus of a real backend, so the prediction and the
//! label finally have independent origins:
//!
//! - **Predictions** come from `phlow-system1`'s rich-answer path —
//!   the `RichAnswer`/`RichAnswerBatch` types that landed with
//!   system1's Clef backend (`crates/phlow-system1/src/clef.rs`) —
//!   served by the estate's only `decide_rich` backend,
//!   [`OllamaBackend`]: real distributions derived from the serving
//!   model's first-token logprobs, fail-closed where no genuine
//!   distribution exists. No Clef HTTP service exists in this
//!   estate; the wire decoder is exercised by its own tests, and
//!   this replay's contract is the rich-answer type, not the
//!   transport.
//! - **Labels** come from an operator-authored labels file whose
//!   entries are derived from each point's (state, question)
//!   semantics only — never from either backend's prediction. A
//!   point with no label entry stays label-pending
//!   (`ground_truth: null`): counted, never scored.
//!
//! # Contract
//!
//! - The source corpus is read, never modified. The output corpus
//!   is a fresh `decision-traces.jsonl` in the output directory;
//!   an existing one is refused, never overwritten or appended to.
//! - Backend failures are data, not silence: every unanswered
//!   point is recorded in `replay-failures.json` with the
//!   backend's own reason, and the summary counts them by kind.
//!   Score points fail closed here — the backend exposes no
//!   per-level score distribution — so Score stays unmeasured
//!   until a backend with a genuine score distribution exists.
//! - The replay writes no `task_outcome` records: it runs no
//!   gauntlet tasks, and the mock run's outcomes describe the
//!   mock run. The calibration report over the new corpus
//!   therefore shows zero task outcomes, honestly.
//! - After writing the corpus, the binary collects it through
//!   `phlow-calibration`'s own collector and renders the design's
//!   report at the design's 10-bin resolution and at 5 bins (the
//!   resolution this corpus size supports), writing both to the
//!   output directory as `report-10bin.txt` / `report-5bin.txt`.
//!
//! Usage:
//! `clef-replay --corpus <decision-traces.jsonl> --labels <labels.json>
//! --out-dir <dir> --model <ollama-tag> [--endpoint <url>]`

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use phlow_calibration::collector::TRACE_FILE_BYTES_MAX;
use phlow_calibration::{CollectionPaths, collect_all, compute_split_metrics, render_report};
use phlow_gauntlet::trace::{DecisionTrace, GroundTruth, TRACE_LINE_BYTES_MAX};
use phlow_system1::{
    DEFAULT_OLLAMA_ENDPOINT, OllamaBackend, OllamaConfig, Question, QuestionBatch, RichAnswerBatch,
    System1Error,
};
use serde_json::Value;

/// Most decision points one replay accepts from a corpus. The real
/// wave produces 142; the bound exists so a stray file cannot turn
/// into an unbounded sequence of model calls.
pub const POINTS_MAX: usize = 4096;

/// Longest backend-failure reason kept in `replay-failures.json`;
/// longer reasons are truncated and marked, never silently cut.
pub const REASON_CHARS_MAX: usize = 512;

/// Longest labels file read (the real one is ~10 KiB).
pub const LABELS_FILE_BYTES_MAX: u64 = 4 * 1024 * 1024;

/// The output corpus file name, the name the collector reads.
pub const TRACES_FILE_NAME: &str = "decision-traces.jsonl";
/// The backend-failure record file name.
pub const FAILURES_FILE_NAME: &str = "replay-failures.json";

/// Command-line arguments, parsed by hand (the gauntlet binary's
/// convention; no CLI framework is a workspace dependency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// The mock-backend corpus to replay (read-only).
    pub corpus: PathBuf,
    /// The endpoint the backend serves on.
    pub endpoint: String,
    /// The operator-authored labels file.
    pub labels: PathBuf,
    /// The Ollama model tag to ask.
    pub model: String,
    /// Directory the new corpus and reports are written to.
    pub out_dir: PathBuf,
}

/// Why a replay could not proceed. Structural failures only:
/// per-point backend failures are data (see the module docs) and
/// travel in the failures file, not in this error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// A required CLI flag is missing or malformed.
    Args {
        /// What was wrong.
        reason: String,
    },
    /// An input violates the replay contract (malformed record,
    /// kind mismatch, unknown or duplicate label, oversize input).
    Contract {
        /// What was wrong.
        reason: String,
    },
    /// A filesystem operation failed.
    Io {
        /// The path involved.
        path: PathBuf,
        /// The OS error text.
        reason: String,
    },
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayError::Args { reason } => write!(f, "clef-replay: {reason}"),
            ReplayError::Contract { reason } => write!(f, "clef-replay: {reason}"),
            ReplayError::Io { path, reason } => {
                write!(f, "clef-replay: {}: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for ReplayError {}

fn contract(reason: String) -> ReplayError {
    ReplayError::Contract { reason }
}

fn io(path: &Path, error: std::io::Error) -> ReplayError {
    ReplayError::Io {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

const USAGE: &str = "usage: clef-replay --corpus PATH --labels PATH \
    --out-dir DIR --model TAG [--endpoint URL]";

/// Parse argv (without the program name).
pub fn parse_args(argv: &[String]) -> Result<Args, ReplayError> {
    let mut corpus: Option<PathBuf> = None;
    let mut endpoint: Option<String> = None;
    let mut labels: Option<PathBuf> = None;
    let mut model: Option<String> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut i = 0_usize;
    while i < argv.len() {
        let flag = argv[i].as_str();
        let value = |i: usize| -> Result<String, ReplayError> {
            argv.get(i + 1).cloned().ok_or_else(|| ReplayError::Args {
                reason: format!("flag {flag} needs a value\n{USAGE}"),
            })
        };
        match flag {
            "--corpus" => corpus = Some(PathBuf::from(value(i)?)),
            "--endpoint" => endpoint = Some(value(i)?),
            "--labels" => labels = Some(PathBuf::from(value(i)?)),
            "--model" => model = Some(value(i)?),
            "--out-dir" => out_dir = Some(PathBuf::from(value(i)?)),
            other => {
                return Err(ReplayError::Args {
                    reason: format!("unknown flag '{other}'\n{USAGE}"),
                });
            }
        }
        i += 2;
    }
    let missing = |name: &str| ReplayError::Args {
        reason: format!("missing required flag {name}\n{USAGE}"),
    };
    Ok(Args {
        corpus: corpus.ok_or_else(|| missing("--corpus"))?,
        endpoint: endpoint.unwrap_or_else(|| DEFAULT_OLLAMA_ENDPOINT.to_string()),
        labels: labels.ok_or_else(|| missing("--labels"))?,
        model: model.ok_or_else(|| missing("--model"))?,
        out_dir: out_dir.ok_or_else(|| missing("--out-dir"))?,
    })
}

/// One decision point lifted from a source corpus: everything a
/// backend needs to re-answer the question, and nothing about what
/// the mock predicted (the mock's answer is deliberately not
/// carried: it must not leak into the new corpus anywhere).
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// The question the point asks.
    pub question: Question,
    /// Id of the question inside its original batch.
    pub question_id: String,
    /// The state string the decision was made against.
    pub state: String,
    /// The gauntlet task that made the decision.
    pub task_id: String,
}

impl Point {
    /// The labels-file key for this point: `task-id/question-id`.
    pub fn key(&self) -> String {
        format!("{}/{}", self.task_id, self.question_id)
    }

    /// The lowercase kind name used in summaries and failures.
    pub fn kind_label(&self) -> &'static str {
        match self.question {
            Question::Choice { .. } => "choice",
            Question::Noul { .. } => "noul",
            Question::Score { .. } => "score",
        }
    }
}

/// Read a file under a byte bound (checked from metadata first).
fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, ReplayError> {
    let metadata = std::fs::metadata(path).map_err(|e| io(path, e))?;
    if metadata.len() > max {
        return Err(contract(format!(
            "{} is {} bytes, over the {max} bound",
            path.display(),
            metadata.len()
        )));
    }
    std::fs::read(path).map_err(|e| io(path, e))
}

/// Extract the decision points from a source corpus, in file
/// order. `task_outcome` records are counted and skipped (they
/// describe the source run, not a question). Fail-closed like the
/// calibration collector: any malformed line rejects the file.
pub fn load_points(path: &Path) -> Result<Vec<Point>, ReplayError> {
    let bytes = read_bounded(path, TRACE_FILE_BYTES_MAX)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| contract(format!("{} is not valid UTF-8", path.display())))?;
    let mut points: Vec<Point> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > TRACE_LINE_BYTES_MAX {
            return Err(contract(format!(
                "{} line {line_no} exceeds the per-line bound",
                path.display()
            )));
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|error| contract(format!("{} line {line_no}: {error}", path.display())))?;
        let kind = value.get("kind").and_then(Value::as_str).ok_or_else(|| {
            contract(format!(
                "{} line {line_no}: no string 'kind'",
                path.display()
            ))
        })?;
        match kind {
            "task_outcome" => {}
            "decision_trace" => {
                let field = |name: &str| -> Result<&Value, ReplayError> {
                    value.get(name).ok_or_else(|| {
                        contract(format!("{} line {line_no}: no '{name}'", path.display()))
                    })
                };
                let text_field = |name: &str| -> Result<String, ReplayError> {
                    field(name)?.as_str().map(str::to_string).ok_or_else(|| {
                        contract(format!(
                            "{} line {line_no}: '{name}' is not a string",
                            path.display()
                        ))
                    })
                };
                let question: Question = serde_json::from_value(field("question")?.clone())
                    .map_err(|error| {
                        contract(format!(
                            "{} line {line_no}: question does not decode: {error}",
                            path.display()
                        ))
                    })?;
                points.push(Point {
                    question,
                    question_id: text_field("question_id")?,
                    state: text_field("state")?,
                    task_id: text_field("task_id")?,
                });
            }
            other => {
                return Err(contract(format!(
                    "{} line {line_no}: unknown record kind '{other}'",
                    path.display()
                )));
            }
        }
    }
    if points.is_empty() {
        return Err(contract(format!(
            "{} holds no decision points",
            path.display()
        )));
    }
    if points.len() > POINTS_MAX {
        return Err(contract(format!(
            "{} holds {} decision points, over the {POINTS_MAX} bound",
            path.display(),
            points.len()
        )));
    }
    Ok(points)
}

/// Parse the labels file: `{"labels": [{"point": "task-id/qid",
/// "truth": {...}}, ...]}` where `truth` has the trace record's
/// ground-truth shape. Duplicate points are rejected.
pub fn load_label_map(path: &Path) -> Result<BTreeMap<String, GroundTruth>, ReplayError> {
    let bytes = read_bounded(path, LABELS_FILE_BYTES_MAX)?;
    let root: Value = serde_json::from_slice(&bytes).map_err(|error| {
        contract(format!(
            "{}: labels file does not parse: {error}",
            path.display()
        ))
    })?;
    let entries = root
        .get("labels")
        .and_then(Value::as_array)
        .ok_or_else(|| contract(format!("{}: no 'labels' array", path.display())))?;
    let mut labels = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let at = format!("{} labels[{index}]", path.display());
        let point = entry
            .get("point")
            .and_then(Value::as_str)
            .ok_or_else(|| contract(format!("{at}: no string 'point'")))?;
        let truth_value = entry
            .get("truth")
            .ok_or_else(|| contract(format!("{at}: no 'truth'")))?;
        let truth =
            parse_truth(truth_value).map_err(|reason| contract(format!("{at}: {reason}")))?;
        if labels.insert(point.to_string(), truth).is_some() {
            return Err(contract(format!("{at}: duplicate label for '{point}'")));
        }
    }
    Ok(labels)
}

/// Decode one ground-truth object (the trace record's shape).
fn parse_truth(value: &Value) -> Result<GroundTruth, String> {
    let tag = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "truth has no string 'type'".to_string())?;
    match tag {
        "Choice" => {
            let raw = value
                .get("correct_index")
                .and_then(Value::as_u64)
                .ok_or_else(|| "choice truth has no 'correct_index'".to_string())?;
            let correct_index =
                usize::try_from(raw).map_err(|_| "correct_index overflows usize".to_string())?;
            Ok(GroundTruth::Choice { correct_index })
        }
        "Noul" => {
            let actually_yes = value
                .get("actually_yes")
                .and_then(Value::as_bool)
                .ok_or_else(|| "noul truth has no 'actually_yes'".to_string())?;
            Ok(GroundTruth::Noul { actually_yes })
        }
        "Score" => {
            let true_value = value
                .get("true_value")
                .and_then(Value::as_f64)
                .ok_or_else(|| "score truth has no 'true_value'".to_string())?;
            Ok(GroundTruth::Score { true_value })
        }
        other => Err(format!("unknown truth type '{other}'")),
    }
}

/// Check every label against the points it will label, before any
/// backend call is spent: each label must name a real point, its
/// kind must match the point's question, a choice index must be in
/// range, and one key must never cover two different questions.
pub fn check_label_map(
    points: &[Point],
    labels: &BTreeMap<String, GroundTruth>,
) -> Result<(), ReplayError> {
    let mut by_key: BTreeMap<String, &Point> = BTreeMap::new();
    for point in points {
        if let Some(first) = by_key.get(&point.key())
            && first.question != point.question
        {
            return Err(contract(format!(
                "points '{}' repeat with different questions; one label cannot cover both",
                point.key()
            )));
        }
        by_key.entry(point.key()).or_insert(point);
    }
    for (key, truth) in labels {
        let Some(point) = by_key.get(key.as_str()) else {
            return Err(contract(format!("label names unknown point '{key}'")));
        };
        let matches = matches!(
            (&point.question, truth),
            (Question::Choice { .. }, GroundTruth::Choice { .. })
                | (Question::Noul { .. }, GroundTruth::Noul { .. })
                | (Question::Score { .. }, GroundTruth::Score { .. })
        );
        if !matches {
            return Err(contract(format!(
                "label for '{key}' has the wrong kind for its question"
            )));
        }
        if let (Question::Choice { options, .. }, GroundTruth::Choice { correct_index }) =
            (&point.question, truth)
            && *correct_index >= options.len()
        {
            return Err(contract(format!(
                "label for '{key}': correct_index {correct_index} is out of range"
            )));
        }
    }
    Ok(())
}

/// One point the backend could not answer, with the backend's own
/// reason (bounded). These are results, not errors: an
/// unanswerable point is a fact about the backend's reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The question kind's lowercase label.
    pub kind: &'static str,
    /// Id of the question inside its original batch.
    pub question_id: String,
    /// The backend's reason, bounded to [`REASON_CHARS_MAX`].
    pub reason: String,
    /// The gauntlet task that made the decision.
    pub task_id: String,
}

/// Bound a backend reason for the failures file.
fn bound_reason(reason: &str) -> String {
    if reason.chars().count() > REASON_CHARS_MAX {
        let kept: String = reason.chars().take(REASON_CHARS_MAX).collect();
        format!("{kept}…[truncated]")
    } else {
        reason.to_string()
    }
}

/// Re-ask every point of `backend` and build the new traces.
/// `answer` is the backend seam (in production,
/// `OllamaBackend::decide_rich`); it is a parameter so the replay
/// logic is testable without a daemon. Answered points become
/// validated [`DecisionTrace`]s carrying the recorded distribution
/// and the point's label (if any); unanswered points become
/// [`Failure`]s.
pub fn replay(
    points: &[Point],
    labels: &BTreeMap<String, GroundTruth>,
    answer: &dyn Fn(&QuestionBatch) -> Result<RichAnswerBatch, System1Error>,
) -> (Vec<DecisionTrace>, Vec<Failure>) {
    let mut traces = Vec::new();
    let mut failures = Vec::new();
    for point in points {
        let batch = QuestionBatch {
            state: point.state.clone(),
            questions: BTreeMap::from([(point.question_id.clone(), point.question.clone())]),
        };
        let failure = |reason: String| Failure {
            kind: point.kind_label(),
            question_id: point.question_id.clone(),
            reason: bound_reason(&reason),
            task_id: point.task_id.clone(),
        };
        let rich = match answer(&batch) {
            Ok(rich) => rich,
            Err(error) => {
                failures.push(failure(error.to_string()));
                continue;
            }
        };
        let Some(rich_answer) = rich.answers.get(&point.question_id) else {
            failures.push(failure(
                "backend returned no answer for the question".to_string(),
            ));
            continue;
        };
        let trace = DecisionTrace::new(
            point.task_id.clone(),
            point.question_id.clone(),
            point.state.clone(),
            point.question.clone(),
            rich_answer.answer,
            Some(rich_answer.distribution.clone()),
            labels.get(&point.key()).copied(),
        );
        match trace.validate() {
            Ok(()) => traces.push(trace),
            Err(error) => failures.push(failure(format!("trace rejected: {error}"))),
        }
    }
    (traces, failures)
}

/// Serialize the traces as the new corpus (one validated JSONL
/// record per trace, recorder format).
pub fn corpus_text(traces: &[DecisionTrace]) -> Result<String, ReplayError> {
    let mut text = String::new();
    for trace in traces {
        let line = trace
            .to_json_line()
            .map_err(|error| contract(format!("trace does not serialize: {error}")))?;
        text.push_str(&line);
        text.push('\n');
    }
    Ok(text)
}

/// Serialize the failures file.
pub fn failures_text(failures: &[Failure]) -> String {
    let records: Vec<Value> = failures
        .iter()
        .map(|failure| {
            serde_json::json!({
                "kind": failure.kind,
                "question_id": failure.question_id,
                "reason": failure.reason,
                "task_id": failure.task_id,
            })
        })
        .collect();
    let mut text =
        serde_json::to_string_pretty(&Value::Array(records)).unwrap_or_else(|_| "[]".to_string());
    text.push('\n');
    text
}

/// Write `text` to `path` atomically (temp file + rename in the
/// same directory). Refuses to replace an existing file: corpora
/// are provenance records, not build outputs.
pub fn write_new_file(path: &Path, text: &str) -> Result<(), ReplayError> {
    if path.exists() {
        return Err(contract(format!(
            "{} already exists; refusing to overwrite a corpus artifact",
            path.display()
        )));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| io(path, e))?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, text).map_err(|e| io(path, e))?;
    std::fs::rename(&temp, path).map_err(|e| io(path, e))?;
    Ok(())
}

/// Count points/traces/failures by kind label, for the summary.
pub fn count_by_kind<'a>(items: impl Iterator<Item = &'a str>) -> BTreeMap<&'a str, usize> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for item in items {
        *counts.entry(item).or_insert(0) += 1;
    }
    counts
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(argv: &[String]) -> Result<String, ReplayError> {
    let args = parse_args(argv)?;
    let points = load_points(&args.corpus)?;
    let labels = load_label_map(&args.labels)?;
    check_label_map(&points, &labels)?;
    let traces_path = args.out_dir.join(TRACES_FILE_NAME);
    if traces_path.exists() {
        return Err(contract(format!(
            "{} already exists; refusing to overwrite a corpus artifact",
            traces_path.display()
        )));
    }
    let config = OllamaConfig {
        endpoint: args.endpoint.clone(),
        model: args.model.clone(),
    };
    let backend = OllamaBackend::new(&config)
        .map_err(|error| contract(format!("backend does not build: {error}")))?;
    let (traces, failures) = replay(&points, &labels, &|batch| backend.decide_rich(batch));
    std::fs::create_dir_all(&args.out_dir).map_err(|e| io(&args.out_dir, e))?;
    write_new_file(&traces_path, &corpus_text(&traces)?)?;
    write_new_file(
        &args.out_dir.join(FAILURES_FILE_NAME),
        &failures_text(&failures),
    )?;
    let reports = render_reports(&args.out_dir, &traces_path)?;
    for (name, report) in &reports {
        write_new_file(&args.out_dir.join(name), report)?;
    }
    Ok(summarize(&args, &points, &traces, &failures, &reports))
}

/// Collect the new corpus through phlow-calibration's own
/// collector and render the design's report at the design's
/// 10-bin resolution and at 5 bins.
fn render_reports(
    out_dir: &Path,
    traces_path: &Path,
) -> Result<Vec<(String, String)>, ReplayError> {
    let paths = CollectionPaths {
        approval_store: out_dir.join("consumed-approvals.jsonl"),
        feedback_db: out_dir.join("feedback.db"),
        trace_file: Some(traces_path.to_path_buf()),
    };
    let collection = collect_all(&paths)
        .map_err(|error| contract(format!("new corpus does not collect: {error}")))?;
    let mut reports = Vec::new();
    for (bins, name) in [(10_usize, "report-10bin.txt"), (5_usize, "report-5bin.txt")] {
        let split = compute_split_metrics(collection.tuples(), bins);
        let mut report = render_report(&collection, &split);
        report = format!("(rendered at {bins} bins)\n\n{report}");
        reports.push((name.to_string(), report));
    }
    Ok(reports)
}

/// The run summary printed to stdout: point/answer/failure
/// accounting by kind, label coverage, artifact paths, and both
/// rendered reports.
fn summarize(
    args: &Args,
    points: &[Point],
    traces: &[DecisionTrace],
    failures: &[Failure],
    reports: &[(String, String)],
) -> String {
    let point_kinds = count_by_kind(points.iter().map(|p| p.kind_label()));
    let fail_kinds = count_by_kind(failures.iter().map(|f| f.kind));
    let labeled = traces.iter().filter(|t| t.ground_truth.is_some()).count();
    let mut out = String::new();
    out.push_str(&format!(
        "points: {} (choice {}, noul {}, score {})\n",
        points.len(),
        point_kinds.get("choice").copied().unwrap_or(0),
        point_kinds.get("noul").copied().unwrap_or(0),
        point_kinds.get("score").copied().unwrap_or(0),
    ));
    out.push_str(&format!(
        "answered: {} ({labeled} labeled, {} label-pending)\n",
        traces.len(),
        traces.len() - labeled,
    ));
    out.push_str(&format!(
        "backend failures: {} (choice {}, noul {}, score {})\n",
        failures.len(),
        fail_kinds.get("choice").copied().unwrap_or(0),
        fail_kinds.get("noul").copied().unwrap_or(0),
        fail_kinds.get("score").copied().unwrap_or(0),
    ));
    out.push_str(&format!("model: {} at {}\n", args.model, args.endpoint));
    out.push_str(&format!(
        "corpus: {}\n",
        args.out_dir.join(TRACES_FILE_NAME).display()
    ));
    for (name, report) in reports {
        out.push_str(&format!("\n===== {name} =====\n{report}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use phlow_system1::{Answer, RichAnswer};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phlow-clef-replay-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn noul_point(task_id: &str, question_id: &str) -> Point {
        Point {
            question: Question::Noul {
                instructions: "Is it reversible?".to_string(),
            },
            question_id: question_id.to_string(),
            state: "state".to_string(),
            task_id: task_id.to_string(),
        }
    }

    fn trace_line(task_id: &str, question_id: &str) -> String {
        let trace = DecisionTrace::new(
            task_id.to_string(),
            question_id.to_string(),
            "state".to_string(),
            Question::Noul {
                instructions: "Is it reversible?".to_string(),
            },
            Answer::Noul {
                yes: true,
                probability: 0.99,
            },
            Some(vec![0.01, 0.99]),
            None,
        );
        trace.to_json_line().unwrap()
    }

    // --- Validation ---

    #[test]
    fn load_points_reads_traces_and_skips_outcomes() {
        let dir = temp_dir("points");
        let path = dir.join("corpus.jsonl");
        let outcome = "{\"held_out\":false,\"kind\":\"task_outcome\",\"passed\":true,\
                       \"recorded_at_unix_ms\":1,\"task_id\":\"task-251\"}";
        std::fs::write(
            &path,
            format!(
                "{}\n{outcome}\n{}\n",
                trace_line("task-251", "q"),
                trace_line("task-252", "q")
            ),
        )
        .unwrap();
        let points = load_points(&path).unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].task_id, "task-251");
        assert_eq!(points[1].kind_label(), "noul");
    }

    #[test]
    fn labels_round_trip_and_check() {
        let dir = temp_dir("labels");
        let path = dir.join("labels.json");
        std::fs::write(
            &path,
            "{\"labels\":[{\"point\":\"task-251/q\",\"truth\":\
             {\"actually_yes\":true,\"type\":\"Noul\"}}]}",
        )
        .unwrap();
        let labels = load_label_map(&path).unwrap();
        let points = vec![noul_point("task-251", "q")];
        check_label_map(&points, &labels).unwrap();
        assert_eq!(
            labels["task-251/q"],
            GroundTruth::Noul { actually_yes: true }
        );
    }

    #[test]
    fn replay_records_labels_and_distributions() {
        let points = vec![noul_point("task-251", "q"), noul_point("task-252", "q")];
        let mut labels = BTreeMap::new();
        labels.insert(
            "task-251/q".to_string(),
            GroundTruth::Noul { actually_yes: true },
        );
        let answer = |batch: &QuestionBatch| -> Result<RichAnswerBatch, System1Error> {
            let mut answers = BTreeMap::new();
            for id in batch.questions.keys() {
                answers.insert(
                    id.clone(),
                    RichAnswer {
                        answer: Answer::Noul {
                            yes: true,
                            probability: 0.8,
                        },
                        distribution: vec![0.2, 0.8],
                    },
                );
            }
            Ok(RichAnswerBatch { answers })
        };
        let (traces, failures) = replay(&points, &labels, &answer);
        assert!(failures.is_empty());
        assert_eq!(traces.len(), 2);
        assert_eq!(
            traces[0].ground_truth,
            Some(GroundTruth::Noul { actually_yes: true })
        );
        assert_eq!(traces[0].predicted_dist, Some(vec![0.2, 0.8]));
        // The unlabeled point stays label-pending, never invented.
        assert_eq!(traces[1].ground_truth, None);
    }

    #[test]
    fn replay_failures_are_counted_not_dropped() {
        let points = vec![noul_point("task-251", "q")];
        let labels = BTreeMap::new();
        let answer = |_batch: &QuestionBatch| -> Result<RichAnswerBatch, System1Error> {
            Err(System1Error::DistributionUnavailable {
                reason: "no distribution for this kind",
            })
        };
        let (traces, failures) = replay(&points, &labels, &answer);
        assert!(traces.is_empty());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].kind, "noul");
        assert!(failures[0].reason.contains("no distribution"));
    }

    #[test]
    fn write_new_file_is_atomic_and_refuses_overwrite() {
        let dir = temp_dir("write");
        let path = dir.join("out.jsonl");
        write_new_file(&path, "line\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "line\n");
        assert!(write_new_file(&path, "other\n").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "line\n");
    }

    // --- Adversarial ---

    #[test]
    fn load_points_rejects_malformed_line() {
        let dir = temp_dir("malformed");
        let path = dir.join("corpus.jsonl");
        std::fs::write(
            &path,
            format!("{}\nnot json\n", trace_line("task-251", "q")),
        )
        .unwrap();
        assert!(load_points(&path).is_err());
    }

    #[test]
    fn load_points_rejects_unknown_record_kind() {
        let dir = temp_dir("kind");
        let path = dir.join("corpus.jsonl");
        std::fs::write(&path, "{\"kind\":\"mystery\"}\n").unwrap();
        assert!(load_points(&path).is_err());
    }

    #[test]
    fn labels_reject_duplicates_and_unknown_points() {
        let dir = temp_dir("duplabels");
        let path = dir.join("labels.json");
        std::fs::write(
            &path,
            "{\"labels\":[
              {\"point\":\"task-251/q\",\"truth\":{\"actually_yes\":true,\"type\":\"Noul\"}},
              {\"point\":\"task-251/q\",\"truth\":{\"actually_yes\":false,\"type\":\"Noul\"}}
            ]}",
        )
        .unwrap();
        assert!(load_label_map(&path).is_err());
        let points = vec![noul_point("task-251", "q")];
        let mut stray = BTreeMap::new();
        stray.insert(
            "task-999/q".to_string(),
            GroundTruth::Noul { actually_yes: true },
        );
        assert!(check_label_map(&points, &stray).is_err());
    }

    #[test]
    fn labels_reject_kind_mismatch_and_out_of_range_index() {
        let points = vec![noul_point("task-251", "q")];
        let mut wrong_kind = BTreeMap::new();
        wrong_kind.insert(
            "task-251/q".to_string(),
            GroundTruth::Choice { correct_index: 0 },
        );
        assert!(check_label_map(&points, &wrong_kind).is_err());
        let choice_point = Point {
            question: Question::Choice {
                instructions: "Pick.".to_string(),
                options: vec!["a".to_string(), "b".to_string()],
            },
            question_id: "q".to_string(),
            state: "state".to_string(),
            task_id: "task-251".to_string(),
        };
        let mut out_of_range = BTreeMap::new();
        out_of_range.insert(
            "task-251/q".to_string(),
            GroundTruth::Choice { correct_index: 2 },
        );
        assert!(check_label_map(&[choice_point], &out_of_range).is_err());
    }

    #[test]
    fn replay_rejects_an_invalid_backend_distribution() {
        // A backend that hands back a vector that is not a genuine
        // distribution must not reach the corpus: the trace contract
        // rejects it and the point lands in the failures, verbatim.
        let points = vec![noul_point("task-251", "q")];
        let labels = BTreeMap::new();
        let answer = |batch: &QuestionBatch| -> Result<RichAnswerBatch, System1Error> {
            let mut answers = BTreeMap::new();
            for id in batch.questions.keys() {
                answers.insert(
                    id.clone(),
                    RichAnswer {
                        answer: Answer::Noul {
                            yes: true,
                            probability: 0.8,
                        },
                        distribution: vec![0.5, 0.8],
                    },
                );
            }
            Ok(RichAnswerBatch { answers })
        };
        let (traces, failures) = replay(&points, &labels, &answer);
        assert!(traces.is_empty());
        assert_eq!(failures.len(), 1);
        assert!(failures[0].reason.contains("trace rejected"));
    }
}
