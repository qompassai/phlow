//! Reviewer calibration (PH-5): is a reviewer's verdict worth
//! anything, measured per model against what actually held up?
//!
//! The runtime records the reviewer's verdict as a label
//! (`approved` / `not_approved`) — no numeric confidence exists, and
//! none is invented here. A binary verdict read as a certain
//! prediction has a Brier score equal to its error rate, so the
//! reliability table reports agreement and error directly, against
//! two truth bases:
//!
//! - **Ground truth** (primary): the corpus manifest's independently
//!   verified outcome for the run. This is where *escapes* show: a
//!   run the checks and the reviewer both passed that was actually
//!   wrong is a false approval here even when the host gate agreed.
//! - **Host gate** (secondary): whether PH-1 recorded the run as
//!   `tool_checked`. This measures agreement with the checks alone.
//!
//! `phlow-calibration` is not the vehicle for this table: its tuples
//! are per-question records carrying protocol distributions, and its
//! own data-honesty doctrine forbids inventing a distribution where
//! none exists. A run verdict has none; the degenerate binary
//! reading above is the honest one.
//!
//! # Demotion (shadow-before-trust)
//!
//! A reviewer model whose ground-truth agreement falls below
//! [`CALIBRATION_FLOOR_AGREEMENT`] or whose false-approval rate
//! exceeds [`FALSE_APPROVAL_CEILING`], over at least
//! [`MIN_CALIBRATION_RUNS`] scored verdicts, is **proposed** for
//! demotion to advisory: its verdict is still recorded, but it no
//! longer gates cycle approval. The demotion is a proposal in the
//! rendered report — config the operator applies by hand, the same
//! posture as PH-2's budgets. Below the minimum run count the table
//! says *insufficient evidence*, which is neither trust nor demotion.

use std::collections::BTreeMap;

use crate::budget::{LabelledRun, Truth};
use crate::outcome::RunOutcome;

/// Ground-truth agreement a reviewer must reach to keep gating.
pub const CALIBRATION_FLOOR_AGREEMENT: f64 = 0.80;
/// False-approval rate above which a reviewer is proposed for
/// demotion regardless of overall agreement (approving bad work is
/// the expensive error).
pub const FALSE_APPROVAL_CEILING: f64 = 0.25;
/// Fewest scored verdicts before any calibration verdict — trust or
/// demotion — is stated for a model.
pub const MIN_CALIBRATION_RUNS: usize = 5;

/// The reviewer's verdict, reduced to the two gating labels. Any
/// other recorded label (a status string, an absent reviewer) is not
/// a verdict and is counted, never scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The reviewer approved the run's result.
    Approved,
    /// The reviewer did not approve.
    NotApproved,
}

impl Verdict {
    /// Parse the outcome row's verdict label.
    pub fn parse(text: Option<&str>) -> Option<Verdict> {
        match text {
            Some("approved") => Some(Verdict::Approved),
            Some("not_approved") => Some(Verdict::NotApproved),
            _ => None,
        }
    }
}

/// The calibration verdict for one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationVerdict {
    /// Enough evidence, at or above the floor: no action proposed.
    Calibrated,
    /// Enough evidence, below the floor: demotion to advisory is
    /// proposed.
    DemoteToAdvisory,
    /// Too few scored verdicts to say anything either way.
    InsufficientEvidence,
}

/// One model's reliability row: verdicts scored against ground truth.
#[derive(Debug, Clone, PartialEq)]
pub struct ReliabilityRow {
    /// Reviewer model id.
    pub model: String,
    /// Scored verdicts (verdict present, ground truth known).
    pub scored: usize,
    /// Verdicts that approved.
    pub approved: usize,
    /// Verdicts that did not approve.
    pub not_approved: usize,
    /// Approved and actually good.
    pub true_approvals: usize,
    /// Approved but actually bad — the expensive error (an escape
    /// past both reviewer and, often, the checks).
    pub false_approvals: usize,
    /// Not approved but actually good.
    pub false_rejections: usize,
    /// Not approved and actually bad.
    pub true_rejections: usize,
    /// `(true approvals + true rejections) / scored`.
    pub agreement: f64,
    /// Error rate; also the Brier score of the verdict read as a
    /// certain prediction.
    pub error_rate: f64,
    /// `false approvals / (false approvals + true rejections)`;
    /// `None` when the model never saw a bad run.
    pub false_approval_rate: Option<f64>,
    /// `false rejections / (false rejections + true approvals)`;
    /// `None` when the model never saw a good run.
    pub false_rejection_rate: Option<f64>,
    /// Agreement with the host gate (`tool_checked`) over the same
    /// scored runs — the checks-only comparison.
    pub host_gate_agreement: f64,
    /// Runs attributed to this model whose verdict was absent or not
    /// a gating label (counted, not scored).
    pub unscored_verdicts: usize,
    /// The calibration verdict for this model.
    pub verdict: CalibrationVerdict,
}

/// Build the reliability table: one row per reviewer model named in
/// the corpus labels, in sorted model order. Runs whose label names
/// no reviewer model are unattributed and excluded (the outcome row
/// itself does not record which model reviewed — see the budget
/// module's manifest rationale).
pub fn reliability_table(runs: &[LabelledRun]) -> Vec<ReliabilityRow> {
    let mut by_model: BTreeMap<String, Vec<&LabelledRun>> = BTreeMap::new();
    for run in runs {
        if let Some(model) = &run.label.reviewer_model {
            by_model.entry(model.clone()).or_default().push(run);
        }
    }
    by_model
        .into_iter()
        .map(|(model, model_runs)| score_model(&model, &model_runs))
        .collect()
}

/// Score one model's runs against ground truth and the host gate.
fn score_model(model: &str, runs: &[&LabelledRun]) -> ReliabilityRow {
    let mut row = ReliabilityRow {
        model: model.to_owned(),
        scored: 0,
        approved: 0,
        not_approved: 0,
        true_approvals: 0,
        false_approvals: 0,
        false_rejections: 0,
        true_rejections: 0,
        agreement: 0.0,
        error_rate: 0.0,
        false_approval_rate: None,
        false_rejection_rate: None,
        host_gate_agreement: 0.0,
        unscored_verdicts: 0,
        verdict: CalibrationVerdict::InsufficientEvidence,
    };
    let mut host_agreements = 0usize;
    for run in runs {
        let Some(verdict) = Verdict::parse(run.row.reviewer_verdict.as_deref()) else {
            row.unscored_verdicts += 1;
            continue;
        };
        row.scored += 1;
        let good = run.label.truth == Truth::Success;
        match (verdict, good) {
            (Verdict::Approved, true) => {
                row.approved += 1;
                row.true_approvals += 1;
            }
            (Verdict::Approved, false) => {
                row.approved += 1;
                row.false_approvals += 1;
            }
            (Verdict::NotApproved, true) => {
                row.not_approved += 1;
                row.false_rejections += 1;
            }
            (Verdict::NotApproved, false) => {
                row.not_approved += 1;
                row.true_rejections += 1;
            }
        }
        let host_good = run.row.host_verified();
        if (verdict == Verdict::Approved) == host_good {
            host_agreements += 1;
        }
    }
    if row.scored > 0 {
        let correct = row.true_approvals + row.true_rejections;
        row.agreement = (correct as f64) / (row.scored as f64);
        row.error_rate = 1.0 - row.agreement;
        row.host_gate_agreement = (host_agreements as f64) / (row.scored as f64);
    }
    let bad_seen = row.false_approvals + row.true_rejections;
    if bad_seen > 0 {
        row.false_approval_rate = Some((row.false_approvals as f64) / (bad_seen as f64));
    }
    let good_seen = row.false_rejections + row.true_approvals;
    if good_seen > 0 {
        row.false_rejection_rate = Some((row.false_rejections as f64) / (good_seen as f64));
    }
    row.verdict = calibration_verdict(&row);
    row
}

/// Apply the stated floor to one scored row.
fn calibration_verdict(row: &ReliabilityRow) -> CalibrationVerdict {
    if row.scored < MIN_CALIBRATION_RUNS {
        return CalibrationVerdict::InsufficientEvidence;
    }
    let false_approval_breach = row
        .false_approval_rate
        .is_some_and(|rate| rate > FALSE_APPROVAL_CEILING);
    if row.agreement < CALIBRATION_FLOOR_AGREEMENT || false_approval_breach {
        CalibrationVerdict::DemoteToAdvisory
    } else {
        CalibrationVerdict::Calibrated
    }
}

/// One run whose recorded verdict differs from its final verdict
/// under the rung-5 semantics (see [`rederive_final_verdicts`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictChange {
    /// The run whose verdict moved.
    pub run_id: String,
    /// The reviewer model the corpus label attributes the run to.
    pub reviewer_model: Option<String>,
    /// Independently verified ground truth for the run.
    pub truth: Truth,
    /// The verdict the outcome row recorded (first reviewer entry,
    /// pre-rung-5 semantics).
    pub recorded: Option<String>,
    /// The run's final verdict (last reviewer entry).
    pub final_verdict: Option<String>,
}

/// Re-derive each run's reviewer verdict from its run report under
/// the rung-5 **final-verdict** semantics, leaving every other field
/// of the row untouched.
///
/// The rung-3 corpus rows were recorded by a runtime whose
/// `from_report` read the *first* reviewer entry of a multi-cycle
/// run; rung 5 corrected the recording to the *last* entry (the
/// verdict the run ended under). The reports themselves carry the
/// full roles log, so the calibration can be re-derived exactly:
/// each run's verdict is recomputed by the same production
/// [`RunOutcome::from_report`] the runtime now uses — never by a
/// second implementation of the rule.
///
/// `reports` maps run id → the run's report JSON (the report's own
/// `outcome_run_id`). A run with no report in the map keeps its
/// recorded verdict and is counted in the returned `missing` tally:
/// absent evidence is disclosed, never filled in.
///
/// Returns the re-derived runs (input order), the verdict changes
/// (input order), and the count of runs without a report.
pub fn rederive_final_verdicts(
    runs: &[LabelledRun],
    reports: &BTreeMap<String, serde_json::Value>,
) -> (Vec<LabelledRun>, Vec<VerdictChange>, usize) {
    let mut rederived = Vec::with_capacity(runs.len());
    let mut changes = Vec::new();
    let mut missing = 0usize;
    for run in runs {
        let Some(report) = reports.get(&run.row.run_id) else {
            missing += 1;
            rederived.push(run.clone());
            continue;
        };
        let final_verdict = RunOutcome::from_report(
            &run.row.run_id,
            &run.label.task_class,
            report,
            run.row.wall_ms,
        )
        .reviewer_verdict;
        if final_verdict != run.row.reviewer_verdict {
            changes.push(VerdictChange {
                run_id: run.row.run_id.clone(),
                reviewer_model: run.label.reviewer_model.clone(),
                truth: run.label.truth,
                recorded: run.row.reviewer_verdict.clone(),
                final_verdict: final_verdict.clone(),
            });
            let mut replaced = run.clone();
            replaced.row.reviewer_verdict = final_verdict;
            rederived.push(replaced);
        } else {
            rederived.push(run.clone());
        }
    }
    (rederived, changes, missing)
}

/// Render the PH-5 report: method, the per-model reliability table,
/// and any demotion proposals as operator-applied config guidance.
/// Rendering is pure — the caller owns the write.
pub fn render_calibration_report(table: &[ReliabilityRow], unattributed_runs: usize) -> String {
    let mut out = String::new();
    out.push_str("# PH-5 — Reviewer calibration (proposal only; the operator applies)\n\n");
    out.push_str(
        "Each reviewer's recorded verdict (`approved` / `not_approved`) is scored \
         against the corpus manifest's independently verified ground truth. The \
         runtime records no numeric confidence, and none is invented: a binary \
         verdict read as a certain prediction has a Brier score equal to its error \
         rate, which is what the table reports. Agreement with the host gate \
         (`tool_checked`) is shown as the checks-only comparison; the gap between \
         the two columns is where escapes live — runs the checks and the reviewer \
         both passed that were actually wrong.\n\n",
    );
    out.push_str(&format!(
        "Floor: agreement >= {CALIBRATION_FLOOR_AGREEMENT:.2} and false-approval rate \
         <= {FALSE_APPROVAL_CEILING:.2}, over at least {MIN_CALIBRATION_RUNS} scored \
         verdicts. Below that count the verdict is *insufficient evidence* — neither \
         trust nor demotion. Runs whose label names no reviewer model: \
         {unattributed_runs} (excluded; the outcome row does not record roles).\n\n"
    ));
    out.push_str("## Reliability per reviewer model\n\n");
    out.push_str(
        "| Model | Scored | Approved | Agreement | Error (=Brier) | False-appr. rate | \
         False-rej. rate | Host-gate agreement | Verdict |\n\
         |---|---|---|---|---|---|---|---|---|\n",
    );
    for row in table {
        out.push_str(&format!(
            "| {} | {} | {} | {:.2} | {:.2} | {} | {} | {:.2} | {} |\n",
            row.model,
            row.scored,
            row.approved,
            row.agreement,
            row.error_rate,
            rate_text(row.false_approval_rate),
            rate_text(row.false_rejection_rate),
            row.host_gate_agreement,
            verdict_text(row.verdict),
        ));
    }
    out.push('\n');
    for row in table {
        render_model_detail(&mut out, row);
    }
    out
}

/// Render an optional rate, or the honest absence of its denominator.
fn rate_text(rate: Option<f64>) -> String {
    match rate {
        Some(value) => format!("{value:.2}"),
        None => "n/a (no such runs)".to_owned(),
    }
}

/// Render a calibration verdict as report text.
fn verdict_text(verdict: CalibrationVerdict) -> &'static str {
    match verdict {
        CalibrationVerdict::Calibrated => "calibrated — no action",
        CalibrationVerdict::DemoteToAdvisory => "**demote to advisory (proposed)**",
        CalibrationVerdict::InsufficientEvidence => "insufficient evidence",
    }
}

/// Render one model's detail section, including the demotion
/// proposal where the floor is breached.
fn render_model_detail(out: &mut String, row: &ReliabilityRow) {
    out.push_str(&format!("### {}\n\n", row.model));
    out.push_str(&format!(
        "Scored verdicts: {} ({} approved, {} not approved); unscored verdicts: {}. \
         True approvals {}, false approvals {}, false rejections {}, true \
         rejections {}.\n\n",
        row.scored,
        row.approved,
        row.not_approved,
        row.unscored_verdicts,
        row.true_approvals,
        row.false_approvals,
        row.false_rejections,
        row.true_rejections,
    ));
    if row.verdict == CalibrationVerdict::DemoteToAdvisory {
        out.push_str(
            "**Demotion proposed (shadow-before-trust).** This reviewer's verdict \
             should stop gating cycle approval and become advisory: the verdict is \
             still recorded on every run (so calibration keeps measuring it), but \
             cycle approval rests on the coder's status and the host verification \
             gate alone until the model's agreement recovers above the floor on a \
             fresh corpus. The operator applies this by hand in the workspace \
             config — the interim form available today is to seat a calibrated \
             model as `reviewer` in `[models]` and keep this model only on \
             non-gating work; the durable form is a `reviewer_mode = \"advisory\"` \
             setting honoured by the runtime's cycle-approval rule, which is \
             proposed here and not implemented by this report.\n\n",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::{CorpusLabel, OutcomeRow, Split};

    fn labelled(
        run_id: &str,
        model: &str,
        verdict: Option<&str>,
        truth: Truth,
        host_verified: bool,
    ) -> LabelledRun {
        LabelledRun {
            row: OutcomeRow {
                run_id: run_id.to_owned(),
                status: "ok".to_owned(),
                reviewer_verdict: verdict.map(str::to_owned),
                cycles: 1,
                tool_calls: 5,
                model_calls: 4,
                wall_ms: 1000,
                checks: Vec::new(),
                verification_strength: if host_verified {
                    "tool_checked".to_owned()
                } else {
                    "self_reported".to_owned()
                },
                model_ids: vec![model.to_owned()],
            },
            label: CorpusLabel {
                run_id: run_id.to_owned(),
                task_class: "analysis".to_owned(),
                split: Split::Fit,
                truth,
                reviewer_model: Some(model.to_owned()),
                note: String::new(),
            },
        }
    }

    #[test]
    fn confusion_counts_and_rates() {
        let runs = vec![
            labelled("1", "rev", Some("approved"), Truth::Success, true),
            labelled("2", "rev", Some("approved"), Truth::Failure, false),
            labelled("3", "rev", Some("not_approved"), Truth::Success, true),
            labelled("4", "rev", Some("not_approved"), Truth::Failure, false),
            labelled("5", "rev", Some("approved"), Truth::Success, true),
            labelled("6", "rev", None, Truth::Success, true),
        ];
        let table = reliability_table(&runs);
        let row = &table[0];
        assert_eq!(row.scored, 5);
        assert_eq!(row.true_approvals, 2);
        assert_eq!(row.false_approvals, 1);
        assert_eq!(row.false_rejections, 1);
        assert_eq!(row.true_rejections, 1);
        assert_eq!(row.unscored_verdicts, 1);
        assert!((row.agreement - 0.6).abs() < 1e-9);
        assert!((row.false_approval_rate.unwrap() - 0.5).abs() < 1e-9);
        // Agreement 0.6 < 0.80 floor over 5 scored -> demotion proposed.
        assert_eq!(row.verdict, CalibrationVerdict::DemoteToAdvisory);
    }

    #[test]
    fn small_sample_is_insufficient_evidence_even_when_wrong() {
        let runs = vec![
            labelled("1", "rev", Some("approved"), Truth::Failure, false),
            labelled("2", "rev", Some("approved"), Truth::Failure, false),
        ];
        let table = reliability_table(&runs);
        assert_eq!(table[0].agreement, 0.0);
        assert_eq!(table[0].verdict, CalibrationVerdict::InsufficientEvidence);
    }

    fn two_cycle_report(first: bool, second: bool) -> serde_json::Value {
        serde_json::json!({
            "status": "failed",
            "verified": false,
            "roles": [
                {"role": "planner", "model": "plan", "status": "ok", "summary": "p"},
                {"role": "coder", "model": "code", "status": "ok", "summary": "v1"},
                {"role": "reviewer", "model": "rev", "status": "ok", "approved": first},
                {"role": "coder", "model": "code", "status": "ok", "summary": "v2"},
                {"role": "reviewer", "model": "rev", "status": "ok", "approved": second}
            ],
            "checks": [],
            "cycles": 2,
            "tool_calls": 0,
            "model_calls": 0
        })
    }

    #[test]
    fn rederive_reads_the_final_verdict_from_the_report() {
        // Rows as the pre-rung-5 runtime recorded them (first-entry
        // semantics): run "ar" approved, run "ra" not approved.
        let runs = vec![
            labelled("ar", "rev", Some("approved"), Truth::Success, false),
            labelled("ra", "rev", Some("not_approved"), Truth::Success, true),
            labelled("same", "rev", Some("approved"), Truth::Success, true),
            labelled("gone", "rev", Some("approved"), Truth::Success, true),
        ];
        let mut reports = BTreeMap::new();
        reports.insert("ar".to_owned(), two_cycle_report(true, false));
        reports.insert("ra".to_owned(), two_cycle_report(false, true));
        reports.insert(
            "same".to_owned(),
            serde_json::json!({
                "status": "ok", "verified": true,
                "roles": [{"role": "reviewer", "model": "rev", "status": "ok", "approved": true}],
                "checks": [], "cycles": 1, "tool_calls": 0, "model_calls": 0
            }),
        );
        let (rederived, changes, missing) = rederive_final_verdicts(&runs, &reports);
        // "gone" has no report: kept as recorded, counted missing.
        assert_eq!(missing, 1);
        assert_eq!(
            rederived[3].row.reviewer_verdict.as_deref(),
            Some("approved")
        );
        // Both two-cycle runs moved; the one-cycle run did not.
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].run_id, "ar");
        assert_eq!(changes[0].recorded.as_deref(), Some("approved"));
        assert_eq!(changes[0].final_verdict.as_deref(), Some("not_approved"));
        assert_eq!(changes[1].run_id, "ra");
        assert_eq!(changes[1].recorded.as_deref(), Some("not_approved"));
        assert_eq!(changes[1].final_verdict.as_deref(), Some("approved"));
        assert_eq!(
            rederived[0].row.reviewer_verdict.as_deref(),
            Some("not_approved")
        );
        assert_eq!(
            rederived[1].row.reviewer_verdict.as_deref(),
            Some("approved")
        );
        assert_eq!(
            rederived[2].row.reviewer_verdict.as_deref(),
            Some("approved")
        );
    }

    #[test]
    fn rederive_can_flip_a_models_calibration_verdict() {
        // Five successful runs, each approved in cycle 1 and rejected
        // in cycle 2 (the corpus F2 pattern): under first-entry
        // semantics the reviewer looks perfect; under final-verdict
        // semantics every verdict is a false rejection and demotion
        // is proposed. The semantics are the measurement.
        let runs: Vec<LabelledRun> = (0..5)
            .map(|i| {
                labelled(
                    &format!("run-{i}"),
                    "rev",
                    Some("approved"),
                    Truth::Success,
                    false,
                )
            })
            .collect();
        let reports: BTreeMap<String, serde_json::Value> = (0..5)
            .map(|i| (format!("run-{i}"), two_cycle_report(true, false)))
            .collect();
        let before = reliability_table(&runs);
        assert_eq!(before[0].agreement, 1.0);
        assert_eq!(before[0].verdict, CalibrationVerdict::Calibrated);
        let (rederived, changes, missing) = rederive_final_verdicts(&runs, &reports);
        assert_eq!(changes.len(), 5);
        assert_eq!(missing, 0);
        let after = reliability_table(&rederived);
        assert_eq!(after[0].agreement, 0.0);
        assert_eq!(after[0].false_rejections, 5);
        assert_eq!(after[0].verdict, CalibrationVerdict::DemoteToAdvisory);
    }

    #[test]
    fn escape_shows_as_false_approval_against_truth_not_host_gate() {
        // Checks and reviewer both passed it; it was actually wrong.
        let runs = (0..5)
            .map(|i| {
                labelled(
                    &format!("run-{i}"),
                    "rev",
                    Some("approved"),
                    Truth::Failure,
                    true,
                )
            })
            .collect::<Vec<_>>();
        let table = reliability_table(&runs);
        let row = &table[0];
        assert_eq!(row.host_gate_agreement, 1.0);
        assert_eq!(row.agreement, 0.0);
        assert_eq!(row.false_approvals, 5);
        assert_eq!(row.verdict, CalibrationVerdict::DemoteToAdvisory);
    }
}
