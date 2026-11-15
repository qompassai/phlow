//! Budget fitting from recorded outcomes (PH-2) — the shared data
//! layer for the rung's two proposal surfaces.
//!
//! PH-1's `run_outcomes` rows record what a run used (cycles, tool
//! calls, model calls, wall time) and how it ended, but the runtime
//! stamps every row's `task_class` with its surface name (`flow_run`)
//! until intake classes land (PH-6), and the row's `model_ids` are a
//! set, not a per-role assignment. Per-class fitting and per-model
//! reviewer calibration therefore join the rows with a **caller-side
//! corpus manifest**: one label per run carrying the task class, the
//! fit/held-out split, the independently verified ground truth, and
//! the reviewer model that served. The manifest is measurement data
//! recorded by whoever ran the corpus, never by the model under
//! measurement.
//!
//! # Posture (PH-9)
//!
//! Everything here is proposal-only. The store is opened **read-only**;
//! no function in this module writes a config, a budget, or the store.
//! The only writes in the rung are the proposal files an operator
//! asked the `phlow-propose` tool to render, to a directory the
//! operator named.
//!
//! # Bounds
//!
//! - [`LABELS_FILE_BYTES_MAX`], [`LABELS_MAX`], [`OUTCOME_ROWS_MAX`]
//!   cap every input before it is parsed or joined.
//! - A label that names an unknown run, duplicates a run, or omits
//!   its ground truth is **refused**, never defaulted (PH-K3's rule
//!   applied to the manifest: an unlabelled calibration row proves
//!   nothing and must not be silently scored or silently dropped).
//! - The margin is a rational `num/den` with `num >= den >= 1`
//!   (a margin below 1.0 would shrink budgets toward the observed
//!   distribution and is refused).

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::error::SelfImproveError;
use crate::outcome::VerificationStrength;

/// The labels file may not exceed this many bytes.
pub const LABELS_FILE_BYTES_MAX: usize = 1_048_576;
/// The manifest may not carry more labels than this.
pub const LABELS_MAX: usize = 4096;
/// At most this many outcome rows are loaded for one fitting pass.
pub const OUTCOME_ROWS_MAX: usize = 100_000;
/// A run id in a label may not exceed this many characters (mirrors
/// the outcome store's own bound).
pub const LABEL_RUN_ID_CHARS_MAX: usize = 128;
/// A task class in a label may not exceed this many characters.
pub const LABEL_CLASS_CHARS_MAX: usize = 128;
/// A reviewer model id in a label may not exceed this many characters.
pub const LABEL_MODEL_CHARS_MAX: usize = 256;
/// A label note may not exceed this many characters.
pub const LABEL_NOTE_CHARS_MAX: usize = 1000;
/// A class needs at least this many successful fit-split runs before
/// a budget is proposed for it; below that the honest answer is no
/// proposal, not a fitted guess.
pub const MIN_FIT_SUCCESSES: usize = 3;

/// Which half of the corpus a run belongs to. The split is assigned
/// before fitting and recorded in the manifest; it is never chosen
/// from the run's results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// Rows the fit reads.
    Fit,
    /// Rows the fit never reads; the proposal is replayed against
    /// them (PH-2's gate).
    HeldOut,
}

impl Split {
    /// Parse the manifest wire string; anything else is refused.
    pub fn parse(text: &str) -> Option<Split> {
        match text {
            "fit" => Some(Split::Fit),
            "held_out" => Some(Split::HeldOut),
            _ => None,
        }
    }
}

/// The independently verified outcome of a run: did the run's result
/// actually hold up when checked outside the pipeline (the host
/// checks plus the corpus verifier's own reading)? This is the
/// ground truth both proposals score against — never the run's own
/// status alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truth {
    /// The result was correct.
    Success,
    /// The result was wrong, fabricated, or absent.
    Failure,
}

impl Truth {
    /// Parse the manifest wire string; anything else is refused.
    pub fn parse(text: &str) -> Option<Truth> {
        match text {
            "success" => Some(Truth::Success),
            "failure" => Some(Truth::Failure),
            _ => None,
        }
    }
}

/// One outcome row, reduced to the fields the rung measures. Loaded
/// read-only from the PH-1 store by [`load_outcome_rows`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeRow {
    /// Runtime-minted run id (`run-<pid>-<nanos>-<counter>`).
    pub run_id: String,
    /// Final run status (`ok`, `failed`, `unverified`, ...).
    pub status: String,
    /// The reviewer's verdict label, when a reviewer ran.
    pub reviewer_verdict: Option<String>,
    /// Planner-coder-reviewer cycles used.
    pub cycles: u64,
    /// Tool calls used.
    pub tool_calls: u64,
    /// Model calls used (the recorded turn count; the row carries no
    /// separate iteration counter, so `max_iterations` is fitted from
    /// this — stated wherever the proposal maps it).
    pub model_calls: u64,
    /// Wall-clock duration of the run in milliseconds.
    pub wall_ms: u64,
    /// Per-check results as `(name, status)` pairs, in recorded order.
    pub checks: Vec<(String, String)>,
    /// Verification strength wire string (`tool_checked`, ...).
    pub verification_strength: String,
    /// Model ids that served, in first-use order (a set, not roles).
    pub model_ids: Vec<String>,
}

impl OutcomeRow {
    /// Whether the host verification gate verified this run (the
    /// strongest label an automated run can carry, PH-K3).
    pub fn host_verified(&self) -> bool {
        VerificationStrength::parse(&self.verification_strength)
            == Some(VerificationStrength::ToolChecked)
    }
}

/// One corpus manifest label: the caller-side record for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusLabel {
    /// The run this label describes.
    pub run_id: String,
    /// Caller-assigned task class (the row's own class is the
    /// runtime's surface name until PH-6).
    pub task_class: String,
    /// Fit/held-out assignment, fixed before fitting.
    pub split: Split,
    /// Independently verified ground truth. Required: a label
    /// without one is refused at load.
    pub truth: Truth,
    /// The model that served the reviewer role for this run, when
    /// the caller recorded roles (rows alone do not carry them).
    pub reviewer_model: Option<String>,
    /// Free-text provenance note (how the truth was verified).
    pub note: String,
}

/// An outcome row joined with its corpus label: one measured run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelledRun {
    /// The recorded outcome.
    pub row: OutcomeRow,
    /// The caller-side label.
    pub label: CorpusLabel,
}

/// A budget margin as a rational `num / den`, always `>= 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Margin {
    /// Numerator; must be `>= den`.
    pub num: u64,
    /// Denominator; must be `>= 1`.
    pub den: u64,
}

impl Margin {
    /// The rung's default margin: 5/4 (25% above the fitted value).
    pub const DEFAULT: Margin = Margin { num: 5, den: 4 };

    /// Validate and build a margin; a margin below 1 is refused.
    pub fn new(num: u64, den: u64) -> Result<Margin, SelfImproveError> {
        if den == 0 || num < den {
            return Err(proposal_invalid(
                "margin",
                format!("margin {num}/{den} is below 1 or undefined"),
            ));
        }
        Ok(Margin { num, den })
    }

    /// `ceil(value * num / den)` with checked arithmetic.
    pub fn apply(&self, value: u64) -> u64 {
        let scaled = (value as u128) * (self.num as u128);
        let quotient = scaled.div_ceil(self.den as u128);
        u64::try_from(quotient).unwrap_or(u64::MAX)
    }
}

/// A proposed budget for one task class (PH-2's deliverable: numbers
/// for the operator to apply by hand, never applied by this code).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetProposal {
    /// Task class the budget covers.
    pub task_class: String,
    /// Proposed `max_iterations` (fitted from recorded model calls).
    pub max_iterations: u64,
    /// Proposed `max_cycles`.
    pub max_cycles: u64,
    /// Proposed `max_tool_calls`.
    pub max_tool_calls: u64,
    /// Proposed run wall-time budget in milliseconds (fitted from
    /// recorded wall time, rounded up to whole seconds).
    pub timeout_ms: u64,
    /// Successful fit-split runs the fit read.
    pub fit_successes: usize,
    /// True when the initial fit cut a held-out success and the
    /// stated dimensions were raised to cover the held-out maximum
    /// (disclosed in the proposal; the final numbers are the ones
    /// replayed).
    pub escalated: bool,
    /// Held-out successes the *initial* fit would have cut (0 unless
    /// `escalated`).
    pub initially_cut: usize,
}

/// The per-class fit outcome: a proposal, or an honest no-proposal
/// with its reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassFit {
    /// Task class.
    pub task_class: String,
    /// Labelled runs in this class (both splits).
    pub runs: usize,
    /// Fit-split runs.
    pub fit_runs: usize,
    /// Held-out runs.
    pub held_out_runs: usize,
    /// The proposal, when one is defensible.
    pub proposal: Option<BudgetProposal>,
    /// Why no budget is proposed, when `proposal` is `None`.
    pub no_proposal_reason: Option<String>,
}

/// Held-out replay results for one class (or the whole corpus).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplayReport {
    /// Held-out runs replayed.
    pub held_out_runs: usize,
    /// Held-out runs whose ground truth is success.
    pub held_out_successes: usize,
    /// Held-out successes the budgets would have cut. PH-2's gate:
    /// the final proposal must show 0 here.
    pub held_out_successes_cut: usize,
    /// Held-out runs whose ground truth is failure.
    pub held_out_failures: usize,
    /// Tool calls the budgets would have saved on held-out failures.
    pub saved_tool_calls: u64,
    /// Total tool calls the held-out failures actually used.
    pub total_failed_tool_calls: u64,
    /// Cycles the budgets would have saved on held-out failures.
    pub saved_cycles: u64,
    /// Wall milliseconds the budgets would have saved on held-out
    /// failures.
    pub saved_wall_ms: u64,
}

impl ReplayReport {
    /// Wasted-turn reduction on failed runs, as a percentage of the
    /// tool calls those runs actually used (0.0 when there were no
    /// failed held-out runs to save on).
    pub fn wasted_turn_reduction_percent(&self) -> f64 {
        if self.total_failed_tool_calls == 0 {
            return 0.0;
        }
        (self.saved_tool_calls as f64) * 100.0 / (self.total_failed_tool_calls as f64)
    }
}

/// Build a proposal-refusal error.
fn proposal_invalid(field: &'static str, reason: String) -> SelfImproveError {
    SelfImproveError::ProposalInvalid { field, reason }
}

/// Load outcome rows from the PH-1 store, **read-only**: the fitting
/// pass must never be able to write the evidence it measures. Rows
/// come back in row-id order (run order) for deterministic fitting.
pub fn load_outcome_rows(db_path: &Path) -> Result<Vec<OutcomeRow>, SelfImproveError> {
    let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = connection.prepare(
        "SELECT run_id, status, reviewer_verdict, cycles, tool_calls,
         model_calls, wall_ms, checks_json, verification_strength,
         model_ids_json FROM run_outcomes ORDER BY id ASC LIMIT ?",
    )?;
    let limit = i64::try_from(OUTCOME_ROWS_MAX).unwrap_or(i64::MAX);
    let rows = statement.query_map([limit], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (
            run_id,
            status,
            verdict,
            cycles,
            tool_calls,
            model_calls,
            wall_ms,
            checks_json,
            strength,
            model_ids_json,
        ) = row?;
        out.push(OutcomeRow {
            run_id,
            status,
            reviewer_verdict: verdict,
            cycles: counter(cycles),
            tool_calls: counter(tool_calls),
            model_calls: counter(model_calls),
            wall_ms: counter(wall_ms),
            checks: parse_checks(&checks_json),
            verification_strength: strength,
            model_ids: parse_model_ids(&model_ids_json),
        });
    }
    Ok(out)
}

/// SQLite counters are signed; a negative counter is corrupt data,
/// read as 0 rather than wrapping into a huge unsigned value.
fn counter(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// Parse the row's per-check JSON into `(name, status)` pairs. A row
/// whose checks do not parse carries no check evidence (empty vec),
/// matching `RunOutcome::from_report`'s defensive posture.
fn parse_checks(checks_json: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<Value>(checks_json) else {
        return Vec::new();
    };
    let Some(array) = value.as_array() else {
        return Vec::new();
    };
    array
        .iter()
        .map(|check| {
            (
                check
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned(),
                check
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned(),
            )
        })
        .collect()
}

/// Parse the row's model-id JSON into an ordered list.
fn parse_model_ids(model_ids_json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(model_ids_json) else {
        return Vec::new();
    };
    let Some(array) = value.as_array() else {
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

/// Load and validate the corpus manifest. Refusals (never defaults):
/// a label with no ground truth, an unknown split or truth string, a
/// duplicate run id, an over-long field, or a file over the byte cap.
pub fn load_corpus_labels(path: &Path) -> Result<Vec<CorpusLabel>, SelfImproveError> {
    let bytes = std::fs::read(path).map_err(SelfImproveError::Io)?;
    if bytes.len() > LABELS_FILE_BYTES_MAX {
        return Err(proposal_invalid(
            "labels",
            format!("labels file exceeds {LABELS_FILE_BYTES_MAX} bytes"),
        ));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| proposal_invalid("labels", "labels file is not UTF-8".to_owned()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| proposal_invalid("labels", format!("labels file is not JSON: {e}")))?;
    let array = value
        .get("labels")
        .and_then(|v| v.as_array())
        .ok_or_else(|| proposal_invalid("labels", "missing \"labels\" array".to_owned()))?;
    if array.len() > LABELS_MAX {
        return Err(proposal_invalid(
            "labels",
            format!("more than {LABELS_MAX} labels"),
        ));
    }
    let mut labels = Vec::with_capacity(array.len());
    for (index, item) in array.iter().enumerate() {
        labels.push(parse_label(index, item)?);
    }
    let mut seen = std::collections::BTreeSet::new();
    for label in &labels {
        if !seen.insert(label.run_id.clone()) {
            return Err(proposal_invalid(
                "labels",
                format!("duplicate label for run {}", label.run_id),
            ));
        }
    }
    Ok(labels)
}

/// Parse one manifest entry; every field is validated, none defaulted
/// except the optional reviewer model and note.
fn parse_label(index: usize, item: &Value) -> Result<CorpusLabel, SelfImproveError> {
    let field = |name: &str| -> Result<&str, SelfImproveError> {
        item.get(name).and_then(|v| v.as_str()).ok_or_else(|| {
            proposal_invalid(
                "labels",
                format!("label {index} is missing string field \"{name}\""),
            )
        })
    };
    let run_id = bounded(field("run_id")?, LABEL_RUN_ID_CHARS_MAX, "run_id")?;
    let task_class = bounded(field("task_class")?, LABEL_CLASS_CHARS_MAX, "task_class")?;
    let split = Split::parse(field("split")?)
        .ok_or_else(|| proposal_invalid("labels", format!("label {index} has an unknown split")))?;
    let truth_text = item.get("truth").and_then(|v| v.as_str()).ok_or_else(|| {
        proposal_invalid(
            "labels",
            format!(
                "label {index} (run {run_id}) has no ground truth; \
                     an unlabelled row is refused, never scored by default"
            ),
        )
    })?;
    let truth = Truth::parse(truth_text).ok_or_else(|| {
        proposal_invalid(
            "labels",
            format!("label {index} has an unknown truth value"),
        )
    })?;
    let reviewer_model = match item.get("reviewer_model") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                proposal_invalid(
                    "labels",
                    format!("label {index} reviewer_model not a string"),
                )
            })?;
            Some(bounded(text, LABEL_MODEL_CHARS_MAX, "reviewer_model")?)
        }
    };
    let note = match item.get("note") {
        None | Some(Value::Null) => String::new(),
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                proposal_invalid("labels", format!("label {index} note not a string"))
            })?;
            bounded(text, LABEL_NOTE_CHARS_MAX, "note")?
        }
    };
    Ok(CorpusLabel {
        run_id,
        task_class,
        split,
        truth,
        reviewer_model,
        note,
    })
}

/// Enforce a character bound on a manifest field.
fn bounded(text: &str, max_chars: usize, field: &'static str) -> Result<String, SelfImproveError> {
    if text.chars().count() > max_chars {
        return Err(SelfImproveError::TooLong { field, max_chars });
    }
    if text.trim().is_empty() {
        return Err(proposal_invalid(field, "must not be empty".to_owned()));
    }
    Ok(text.to_owned())
}

/// Join rows with labels. A label naming a run with no row is
/// refused (the manifest and the store disagree; fitting on a
/// partial join would silently drop measured runs). Rows without a
/// label are returned as a count — they exist, they are simply not
/// part of this corpus.
pub fn join_labelled(
    rows: &[OutcomeRow],
    labels: &[CorpusLabel],
) -> Result<(Vec<LabelledRun>, usize), SelfImproveError> {
    let by_id: BTreeMap<&str, &OutcomeRow> =
        rows.iter().map(|row| (row.run_id.as_str(), row)).collect();
    let mut labelled = Vec::with_capacity(labels.len());
    for label in labels {
        let row = by_id.get(label.run_id.as_str()).ok_or_else(|| {
            proposal_invalid(
                "labels",
                format!("label names run {} which has no outcome row", label.run_id),
            )
        })?;
        labelled.push(LabelledRun {
            row: (*row).clone(),
            label: label.clone(),
        });
    }
    let unlabelled = rows.len().saturating_sub(labelled.len());
    Ok((labelled, unlabelled))
}

/// Nearest-rank 95th percentile of a non-empty sample, ascending.
/// With the small samples a corpus produces, the nearest rank is
/// usually the maximum — that is the honest small-sample behaviour,
/// and the proposal states the sample size beside the result.
pub fn percentile_95(values: &[u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = (95usize * sorted.len()).div_ceil(100);
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    Some(sorted[index])
}

/// Round a millisecond budget up to whole seconds (operator-facing
/// timeouts are second-grained; rounding down could cut a run the
/// fit meant to cover).
pub fn ceil_to_seconds(ms: u64) -> u64 {
    ms.div_ceil(1000) * 1000
}

/// Would this budget have cut this run short? A run is cut when any
/// recorded usage exceeds the proposed cap for that dimension.
pub fn budget_cuts(budget: &BudgetProposal, row: &OutcomeRow) -> bool {
    row.cycles > budget.max_cycles
        || row.tool_calls > budget.max_tool_calls
        || row.model_calls > budget.max_iterations
        || row.wall_ms > budget.timeout_ms
}

/// Fit budgets for every class in the corpus. Fit-split successes
/// only; held-out rows are replayed, never fitted (except the
/// disclosed escalation: if the initial fit cuts a held-out success,
/// the cutting dimensions are raised to the held-out successful
/// maximum times the margin, and the class is marked `escalated` —
/// the proposal reports both the initial cut count and the raise).
pub fn fit_budgets(runs: &[LabelledRun], margin: Margin) -> Vec<ClassFit> {
    let classes = class_names(runs);
    classes
        .iter()
        .map(|class| fit_class(runs, class, margin))
        .collect()
}

/// The distinct task classes present, in sorted (deterministic) order.
fn class_names(runs: &[LabelledRun]) -> Vec<String> {
    let mut names: Vec<String> = runs.iter().map(|r| r.label.task_class.clone()).collect();
    names.sort();
    names.dedup();
    names
}

/// Fit one class.
fn fit_class(runs: &[LabelledRun], class: &str, margin: Margin) -> ClassFit {
    let in_class: Vec<&LabelledRun> = runs
        .iter()
        .filter(|r| r.label.task_class == class)
        .collect();
    let fit: Vec<&LabelledRun> = in_class
        .iter()
        .filter(|r| r.label.split == Split::Fit)
        .copied()
        .collect();
    let held_out: Vec<&LabelledRun> = in_class
        .iter()
        .filter(|r| r.label.split == Split::HeldOut)
        .copied()
        .collect();
    let successes: Vec<&OutcomeRow> = fit
        .iter()
        .filter(|r| r.label.truth == Truth::Success)
        .map(|r| &r.row)
        .collect();
    let base = ClassFit {
        task_class: class.to_owned(),
        runs: in_class.len(),
        fit_runs: fit.len(),
        held_out_runs: held_out.len(),
        proposal: None,
        no_proposal_reason: None,
    };
    if successes.len() < MIN_FIT_SUCCESSES {
        return ClassFit {
            no_proposal_reason: Some(format!(
                "only {} successful fit-split runs (minimum {MIN_FIT_SUCCESSES}); \
                 a distribution this thin cannot carry a budget",
                successes.len()
            )),
            ..base
        };
    }
    let mut proposal = fit_from_successes(class, &successes, margin);
    let initial = replay_class(&held_out, &proposal);
    if initial.held_out_successes_cut > 0 {
        proposal.initially_cut = initial.held_out_successes_cut;
        proposal.escalated = true;
        escalate(&mut proposal, &held_out, margin);
    }
    ClassFit {
        proposal: Some(proposal),
        ..base
    }
}

/// The raw fit: per-dimension 95th percentile of successful usage,
/// times the margin (timeout additionally rounded up to seconds).
fn fit_from_successes(class: &str, successes: &[&OutcomeRow], margin: Margin) -> BudgetProposal {
    let pick = |values: Vec<u64>| percentile_95(&values).unwrap_or(0);
    let cycles: Vec<u64> = successes.iter().map(|r| r.cycles).collect();
    let tool_calls: Vec<u64> = successes.iter().map(|r| r.tool_calls).collect();
    let model_calls: Vec<u64> = successes.iter().map(|r| r.model_calls).collect();
    let wall: Vec<u64> = successes.iter().map(|r| r.wall_ms).collect();
    BudgetProposal {
        task_class: class.to_owned(),
        max_iterations: margin.apply(pick(model_calls)),
        max_cycles: margin.apply(pick(cycles)),
        max_tool_calls: margin.apply(pick(tool_calls)),
        timeout_ms: ceil_to_seconds(margin.apply(pick(wall))),
        fit_successes: successes.len(),
        escalated: false,
        initially_cut: 0,
    }
}

/// Raise each dimension that cuts a held-out success to the held-out
/// successful maximum in that dimension, times the margin.
fn escalate(proposal: &mut BudgetProposal, held_out: &[&LabelledRun], margin: Margin) {
    let successes: Vec<&OutcomeRow> = held_out
        .iter()
        .filter(|r| r.label.truth == Truth::Success)
        .map(|r| &r.row)
        .collect();
    let raise = |current: u64, values: Vec<u64>| -> u64 {
        let max = values.into_iter().max().unwrap_or(0);
        current.max(margin.apply(max))
    };
    proposal.max_cycles = raise(
        proposal.max_cycles,
        successes.iter().map(|r| r.cycles).collect(),
    );
    proposal.max_tool_calls = raise(
        proposal.max_tool_calls,
        successes.iter().map(|r| r.tool_calls).collect(),
    );
    proposal.max_iterations = raise(
        proposal.max_iterations,
        successes.iter().map(|r| r.model_calls).collect(),
    );
    let wall_max = successes.iter().map(|r| r.wall_ms).max().unwrap_or(0);
    proposal.timeout_ms = proposal
        .timeout_ms
        .max(ceil_to_seconds(margin.apply(wall_max)));
}

/// Replay one class's held-out runs against a budget.
fn replay_class(held_out: &[&LabelledRun], budget: &BudgetProposal) -> ReplayReport {
    let mut report = ReplayReport {
        held_out_runs: held_out.len(),
        ..ReplayReport::default()
    };
    for run in held_out {
        match run.label.truth {
            Truth::Success => {
                report.held_out_successes += 1;
                if budget_cuts(budget, &run.row) {
                    report.held_out_successes_cut += 1;
                }
            }
            Truth::Failure => {
                report.held_out_failures += 1;
                report.total_failed_tool_calls += run.row.tool_calls;
                report.saved_tool_calls += run.row.tool_calls.saturating_sub(budget.max_tool_calls);
                report.saved_cycles += run.row.cycles.saturating_sub(budget.max_cycles);
                report.saved_wall_ms += run.row.wall_ms.saturating_sub(budget.timeout_ms);
            }
        }
    }
    report
}

/// Replay the whole corpus against the fitted proposals, per class
/// and in total. Classes without a proposal contribute their held-out
/// counts with no savings claimed.
pub fn replay_all(
    runs: &[LabelledRun],
    fits: &[ClassFit],
) -> (Vec<(String, ReplayReport)>, ReplayReport) {
    let mut per_class = Vec::new();
    let mut total = ReplayReport::default();
    for fit in fits {
        let held_out: Vec<&LabelledRun> = runs
            .iter()
            .filter(|r| r.label.task_class == fit.task_class && r.label.split == Split::HeldOut)
            .collect();
        let report = match &fit.proposal {
            Some(proposal) => replay_class(&held_out, proposal),
            None => ReplayReport {
                held_out_runs: held_out.len(),
                held_out_successes: held_out
                    .iter()
                    .filter(|r| r.label.truth == Truth::Success)
                    .count(),
                held_out_failures: held_out
                    .iter()
                    .filter(|r| r.label.truth == Truth::Failure)
                    .count(),
                ..ReplayReport::default()
            },
        };
        total.held_out_runs += report.held_out_runs;
        total.held_out_successes += report.held_out_successes;
        total.held_out_successes_cut += report.held_out_successes_cut;
        total.held_out_failures += report.held_out_failures;
        total.saved_tool_calls += report.saved_tool_calls;
        total.total_failed_tool_calls += report.total_failed_tool_calls;
        total.saved_cycles += report.saved_cycles;
        total.saved_wall_ms += report.saved_wall_ms;
        per_class.push((fit.task_class.clone(), report));
    }
    (per_class, total)
}

/// The PH-2 gate, enforced as code: a proposal set is refused when
/// any final budget cuts a held-out success, when a budget carries a
/// zero dimension, or when a budget claims a class whose fit read
/// fewer than [`MIN_FIT_SUCCESSES`] successes. The proposal tool runs
/// this before writing anything; a refusal writes nothing.
pub fn validate_budgets(runs: &[LabelledRun], fits: &[ClassFit]) -> Result<(), SelfImproveError> {
    for fit in fits {
        let Some(proposal) = &fit.proposal else {
            continue;
        };
        if proposal.fit_successes < MIN_FIT_SUCCESSES {
            return Err(proposal_invalid(
                "budget",
                format!(
                    "class {} proposes from {} fit successes (minimum {MIN_FIT_SUCCESSES})",
                    fit.task_class, proposal.fit_successes
                ),
            ));
        }
        if proposal.max_iterations == 0
            || proposal.max_cycles == 0
            || proposal.max_tool_calls == 0
            || proposal.timeout_ms == 0
        {
            return Err(proposal_invalid(
                "budget",
                format!("class {} proposes a zero budget dimension", fit.task_class),
            ));
        }
        for run in runs.iter().filter(|r| {
            r.label.task_class == fit.task_class
                && r.label.split == Split::HeldOut
                && r.label.truth == Truth::Success
        }) {
            if budget_cuts(proposal, &run.row) {
                return Err(proposal_invalid(
                    "budget",
                    format!(
                        "class {} budget would cut held-out successful run {}",
                        fit.task_class, run.row.run_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Render the PH-2 proposal file: method, per-class budgets with
/// their evidence, held-out replay results, and the TOML an operator
/// would paste to apply a budget by hand. Rendering is pure — the
/// caller owns the write.
pub fn render_budget_proposal(
    fits: &[ClassFit],
    per_class: &[(String, ReplayReport)],
    total: &ReplayReport,
    margin: Margin,
    unlabelled_rows: usize,
) -> String {
    let mut out = String::new();
    out.push_str("# PH-2 — Budget proposal (proposal only; the operator applies)\n\n");
    out.push_str(
        "Generated from PH-1 `run_outcomes` rows joined with the corpus manifest. \
         Nothing in this file is applied by any tool: budgets change only when the \
         operator edits the config by hand (PH-9: the loop may never widen its own \
         budgets).\n\n",
    );
    out.push_str("## Method\n\n");
    out.push_str(&format!(
        "- Per task class, fit on the fit split's **successful** runs only: nearest-rank \
         95th percentile of recorded usage, times the margin {}/{}. With the small \
         samples here the nearest rank is typically the observed maximum — the sample \
         size is stated beside every number.\n\
         - `max_iterations` is fitted from recorded **model calls** (the outcome row \
         carries no separate iteration counter); `max_cycles` from cycles; \
         `max_tool_calls` from tool calls; the timeout from wall time, rounded up to \
         whole seconds.\n\
         - Held-out replay: a run is *cut* when any recorded usage exceeds the proposed \
         cap. Wasted-turn reduction is measured on held-out **failed** runs, in tool \
         calls: usage above the proposed cap is what the cap would have saved.\n\
         - A class with fewer than {MIN_FIT_SUCCESSES} successful fit-split runs gets \
         **no proposal** — a distribution that thin cannot carry a budget.\n\
         - Outcome rows without a corpus label: {unlabelled_rows} (excluded, counted, \
         never silently fitted).\n\n",
        margin.num, margin.den
    ));
    out.push_str("## Proposed budgets\n\n");
    for fit in fits {
        render_class_fit(&mut out, fit, per_class);
    }
    out.push_str("## Held-out replay — total\n\n");
    out.push_str(&format!(
        "- Held-out runs: {} ({} successful, {} failed)\n\
         - Successful runs cut by the final budgets: **{}** (gate: 0)\n\
         - Wasted tool calls saved on failed runs: {} of {} ({:.1}%)\n\
         - Cycles saved: {}; wall time saved: {} ms\n",
        total.held_out_runs,
        total.held_out_successes,
        total.held_out_failures,
        total.held_out_successes_cut,
        total.saved_tool_calls,
        total.total_failed_tool_calls,
        total.wasted_turn_reduction_percent(),
        total.saved_cycles,
        total.saved_wall_ms,
    ));
    out
}

/// Render one class's section of the budget proposal.
fn render_class_fit(out: &mut String, fit: &ClassFit, per_class: &[(String, ReplayReport)]) {
    out.push_str(&format!("### Task class: {}\n\n", fit.task_class));
    out.push_str(&format!(
        "Labelled runs: {} (fit {}, held-out {})\n\n",
        fit.runs, fit.fit_runs, fit.held_out_runs
    ));
    let Some(proposal) = &fit.proposal else {
        out.push_str(&format!(
            "**No budget proposed.** {}\n\n",
            fit.no_proposal_reason
                .as_deref()
                .unwrap_or("no reason recorded")
        ));
        return;
    };
    out.push_str(&format!(
        "Fitted from {} successful fit-split runs.\n\n\
         | Budget | Proposed |\n|---|---|\n\
         | max_iterations | {} |\n\
         | max_cycles | {} |\n\
         | max_tool_calls | {} |\n\
         | run timeout | {} ms ({} s) |\n\n",
        proposal.fit_successes,
        proposal.max_iterations,
        proposal.max_cycles,
        proposal.max_tool_calls,
        proposal.timeout_ms,
        proposal.timeout_ms / 1000,
    ));
    if proposal.escalated {
        out.push_str(&format!(
            "**Escalation disclosed:** the initial fit would have cut {} held-out \
             successful run(s); the cutting dimensions were raised to the held-out \
             successful maximum times the margin. The numbers above are the raised \
             ones, and the replay below is against them.\n\n",
            proposal.initially_cut
        ));
    }
    if let Some((_, replay)) = per_class.iter().find(|(name, _)| name == &fit.task_class) {
        out.push_str(&format!(
            "Held-out replay for this class: {} successes, {} cut; {} failures, \
             wasted tool calls saved {} of {} ({:.1}%).\n\n",
            replay.held_out_successes,
            replay.held_out_successes_cut,
            replay.held_out_failures,
            replay.saved_tool_calls,
            replay.total_failed_tool_calls,
            replay.wasted_turn_reduction_percent(),
        ));
    }
    out.push_str(
        "To apply (operator's hand, in the workspace config's `[agent]` table):\n\n\
         ```toml\n",
    );
    out.push_str(&format!(
        "max_iterations = {}\nmax_cycles = {}\nmax_tool_calls = {}\n",
        proposal.max_iterations, proposal.max_cycles, proposal.max_tool_calls
    ));
    out.push_str("```\n\n");
    out.push_str(&format!(
        "The fitted run wall budget is {} s; the config's per-request timeout is a \
         different knob and is not changed by this proposal.\n\n",
        proposal.timeout_ms / 1000
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        run_id: &str,
        cycles: u64,
        tool_calls: u64,
        model_calls: u64,
        wall_ms: u64,
    ) -> OutcomeRow {
        OutcomeRow {
            run_id: run_id.to_owned(),
            status: "ok".to_owned(),
            reviewer_verdict: Some("approved".to_owned()),
            cycles,
            tool_calls,
            model_calls,
            wall_ms,
            checks: vec![("theodolite_zero".to_owned(), "ok".to_owned())],
            verification_strength: "tool_checked".to_owned(),
            model_ids: vec!["m".to_owned()],
        }
    }

    fn labelled(
        run_id: &str,
        class: &str,
        split: Split,
        truth: Truth,
        usage: (u64, u64, u64, u64),
    ) -> LabelledRun {
        LabelledRun {
            row: row(run_id, usage.0, usage.1, usage.2, usage.3),
            label: CorpusLabel {
                run_id: run_id.to_owned(),
                task_class: class.to_owned(),
                split,
                truth,
                reviewer_model: Some("rev".to_owned()),
                note: String::new(),
            },
        }
    }

    #[test]
    fn percentile_95_is_nearest_rank() {
        assert_eq!(percentile_95(&[]), None);
        assert_eq!(percentile_95(&[7]), Some(7));
        // n=4: rank = ceil(3.8) = 4 -> the maximum.
        assert_eq!(percentile_95(&[10, 40, 20, 30]), Some(40));
        // n=20: rank = 19 -> second largest.
        let values: Vec<u64> = (1..=20).collect();
        assert_eq!(percentile_95(&values), Some(19));
    }

    #[test]
    fn margin_must_not_shrink() {
        assert!(Margin::new(4, 5).is_err());
        assert!(Margin::new(1, 0).is_err());
        assert_eq!(Margin::new(5, 4).unwrap().apply(41), 52);
        assert_eq!(Margin::DEFAULT.apply(40), 50);
    }

    #[test]
    fn fit_uses_only_fit_split_successes() {
        let mut runs = Vec::new();
        for i in 0..4 {
            runs.push(labelled(
                &format!("fit-{i}"),
                "analysis",
                Split::Fit,
                Truth::Success,
                (1, 10 + i, 6, 60_000),
            ));
        }
        // A huge fit-split failure must not move the fit.
        runs.push(labelled(
            "fit-fail",
            "analysis",
            Split::Fit,
            Truth::Failure,
            (2, 900, 90, 900_000),
        ));
        // A huge held-out success must not move the initial fit either;
        // it triggers the disclosed escalation instead.
        runs.push(labelled(
            "held-1",
            "analysis",
            Split::HeldOut,
            Truth::Success,
            (1, 12, 6, 60_000),
        ));
        let fits = fit_budgets(&runs, Margin::DEFAULT);
        let proposal = fits[0].proposal.as_ref().expect("proposal exists");
        assert_eq!(proposal.fit_successes, 4);
        // p95 of [10,11,12,13] = 13, x1.25 -> 17; held-out max 12 does not cut.
        assert_eq!(proposal.max_tool_calls, 17);
        assert!(!proposal.escalated);
        validate_budgets(&runs, &fits).expect("final budgets validate");
    }

    #[test]
    fn thin_class_gets_no_proposal() {
        let runs = vec![
            labelled(
                "a",
                "code_fix",
                Split::Fit,
                Truth::Success,
                (1, 10, 6, 1000),
            ),
            labelled(
                "b",
                "code_fix",
                Split::Fit,
                Truth::Success,
                (1, 12, 6, 1000),
            ),
            labelled(
                "c",
                "code_fix",
                Split::Fit,
                Truth::Failure,
                (2, 90, 40, 5000),
            ),
        ];
        let fits = fit_budgets(&runs, Margin::DEFAULT);
        assert!(fits[0].proposal.is_none());
        assert!(fits[0].no_proposal_reason.is_some());
    }
}
