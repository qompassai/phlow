//! `phlow-recalibrate` — re-derive the PH-5 reviewer calibration
//! under final-verdict semantics (rung 5) and render a dated
//! addendum beside the original proposal.
//!
//! Usage:
//!     phlow-recalibrate --db PATH --labels PATH --reports-dir PATH
//!                       --date YYYY-MM-DD --out PATH
//!
//! The rung-3 calibration scored the `reviewer_verdict` field as
//! the pre-rung-5 runtime recorded it: the *first* reviewer entry
//! of a multi-cycle run. Rung 5 corrected the recording to the
//! *final* verdict (the last reviewer entry — the verdict the run
//! ended under). This tool recomputes every corpus run's verdict
//! from its run report with the production extraction
//! (`RunOutcome::from_report`, via `rederive_final_verdicts`),
//! rebuilds the reliability table, and renders an addendum that
//! shows both tables and every verdict that moved.
//!
//! It never edits the original proposal document and writes
//! nothing but `--out`: the store is read read-only, the reports
//! are read, and where the addendum and the original disagree the
//! addendum governs — stated in the addendum itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use phlow_self_improve::budget::{
    join_labelled, load_corpus_labels, load_outcome_rows,
};
use phlow_self_improve::reviewer_cal::{
    CalibrationVerdict, ReliabilityRow, rederive_final_verdicts, reliability_table,
};
use phlow_self_improve::{LabelledRun, VerdictChange};

/// Most report files one scan indexes.
const REPORT_FILES_MAX: usize = 10_000;
/// Largest single report file read (run reports carry full role
/// summaries and check output; the cap is generous and explicit).
const REPORT_BYTES_MAX: u64 = 64 * 1024 * 1024;

/// Parsed command line.
struct Args {
    db: PathBuf,
    labels: PathBuf,
    reports_dir: PathBuf,
    date: String,
    out: PathBuf,
}

/// Parse argv by hand (bounded, explicit): unknown or missing
/// arguments are usage errors, never guesses.
fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut db: Option<PathBuf> = None;
    let mut labels: Option<PathBuf> = None;
    let mut reports_dir: Option<PathBuf> = None;
    let mut date: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut index = 0;
    while index < argv.len() {
        let flag = argv[index].as_str();
        let value = |index: &mut usize| -> Result<String, String> {
            *index += 1;
            argv.get(*index)
                .cloned()
                .ok_or_else(|| format!("flag {flag} needs a value"))
        };
        match flag {
            "--db" => db = Some(PathBuf::from(value(&mut index)?)),
            "--labels" => labels = Some(PathBuf::from(value(&mut index)?)),
            "--reports-dir" => reports_dir = Some(PathBuf::from(value(&mut index)?)),
            "--date" => date = Some(value(&mut index)?),
            "--out" => out = Some(PathBuf::from(value(&mut index)?)),
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }
    let date = date.ok_or_else(|| "--date is required".to_owned())?;
    if !is_iso_date(&date) {
        return Err(format!("--date must be YYYY-MM-DD, got {date:?}"));
    }
    Ok(Args {
        db: db.ok_or_else(|| "--db is required".to_owned())?,
        labels: labels.ok_or_else(|| "--labels is required".to_owned())?,
        reports_dir: reports_dir.ok_or_else(|| "--reports-dir is required".to_owned())?,
        date,
        out: out.ok_or_else(|| "--out is required".to_owned())?,
    })
}

/// The `YYYY-MM-DD` shape, checked structurally (the operator dates
/// the addendum; the tool does not invent a calendar).
fn is_iso_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
}

/// Index a reports directory by the reports' own `outcome_run_id`.
/// Files that are unreadable, unparseable, oversize, or carry no
/// run id are counted and skipped — disclosed in the summary, never
/// silently treated as absent runs.
fn index_reports(dir: &Path) -> Result<(BTreeMap<String, serde_json::Value>, usize), String> {
    let mut index = BTreeMap::new();
    let mut skipped = 0usize;
    let mut seen = 0usize;
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read reports dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("reports dir entry failed: {e}"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(".report.json") {
            continue;
        }
        seen += 1;
        if seen > REPORT_FILES_MAX {
            return Err(format!("reports dir exceeds {REPORT_FILES_MAX} report files"));
        }
        let path = entry.path();
        let oversize = path
            .metadata()
            .map(|m| m.len() > REPORT_BYTES_MAX)
            .unwrap_or(false);
        let parsed = if oversize {
            None
        } else {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        };
        match parsed
            .as_ref()
            .and_then(|report| report.get("outcome_run_id"))
            .and_then(|id| id.as_str())
        {
            Some(run_id) => {
                index.insert(run_id.to_owned(), parsed.expect("parsed report"));
            }
            None => skipped += 1,
        }
    }
    Ok((index, skipped))
}

/// A calibration verdict as addendum text.
fn verdict_text(verdict: CalibrationVerdict) -> &'static str {
    match verdict {
        CalibrationVerdict::Calibrated => "calibrated",
        CalibrationVerdict::DemoteToAdvisory => "demote to advisory (proposed)",
        CalibrationVerdict::InsufficientEvidence => "insufficient evidence",
    }
}

/// Render the dated addendum: method, the verdict changes, both
/// reliability tables side by side, and the status-change verdict.
/// Pure — the caller owns the write.
fn render_addendum(
    date: &str,
    runs: &[LabelledRun],
    changes: &[VerdictChange],
    missing_reports: usize,
    skipped_report_files: usize,
    recorded: &[ReliabilityRow],
    final_table: &[ReliabilityRow],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# PH-5 — Reviewer calibration addendum ({date}): final-verdict semantics\n\n"
    ));
    out.push_str(
        "The rung-3 calibration (`reviewer-calibration.md`) scored the outcome \
         rows' `reviewer_verdict` field as the pre-rung-5 runtime recorded it: \
         the **first** reviewer entry of a multi-cycle run. Rung 5 corrected \
         `RunOutcome::from_report` to record the **final** verdict — the last \
         reviewer entry, the verdict the run actually ended under — because a \
         run approved in cycle 1 and rejected in cycle 2 was recorded \
         `approved` while ending `failed`. This addendum re-derives the table \
         over the same corpus, the same labels, and the same ground truth, \
         recomputing each run's verdict from its run report with the \
         production extraction. **The rung-3 document is unchanged; where \
         the two disagree, this addendum governs.** Demotion remains a \
         proposal the operator applies by hand.\n\n",
    );
    out.push_str(&format!(
        "Corpus runs scored: {}. Verdicts recomputed from reports: {}; \
         runs with no report file (recorded verdict kept, disclosed): {}; \
         report files skipped as unreadable or id-less: {}.\n\n",
        runs.len(),
        runs.len() - missing_reports,
        missing_reports,
        skipped_report_files,
    ));
    out.push_str("## Verdict changes\n\n");
    if changes.is_empty() {
        out.push_str("No run's verdict moved under the final-verdict semantics.\n\n");
    } else {
        out.push_str(
            "| Run | Reviewer model | Truth | Recorded (first entry) | Final (last entry) |\n\
             |---|---|---|---|---|\n",
        );
        for change in changes {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                change.run_id,
                change.reviewer_model.as_deref().unwrap_or("(unattributed)"),
                match change.truth {
                    phlow_self_improve::Truth::Success => "success",
                    phlow_self_improve::Truth::Failure => "failure",
                },
                change.recorded.as_deref().unwrap_or("(none)"),
                change.final_verdict.as_deref().unwrap_or("(none)"),
            ));
        }
        out.push('\n');
    }
    out.push_str("## Reliability: recorded vs final\n\n");
    out.push_str(
        "| Model | Scored | Agreement (recorded) | Agreement (final) | \
         False rejections (recorded → final) | Verdict (recorded) | Verdict (final) |\n\
         |---|---|---|---|---|---|---|\n",
    );
    let final_by_model: BTreeMap<&str, &ReliabilityRow> = final_table
        .iter()
        .map(|row| (row.model.as_str(), row))
        .collect();
    for row in recorded {
        let fin = final_by_model.get(row.model.as_str());
        out.push_str(&format!(
            "| {} | {} | {:.2} | {} | {} → {} | {} | {} |\n",
            row.model,
            row.scored,
            row.agreement,
            fin.map(|f| format!("{:.2}", f.agreement))
                .unwrap_or_else(|| "n/a".to_owned()),
            row.false_rejections,
            fin.map(|f| f.false_rejections.to_string())
                .unwrap_or_else(|| "n/a".to_owned()),
            verdict_text(row.verdict),
            fin.map(|f| verdict_text(f.verdict))
                .unwrap_or("(missing)"),
        ));
    }
    out.push('\n');
    out.push_str("## Status changes\n\n");
    let mut any_change = false;
    for row in recorded {
        if let Some(fin) = final_by_model.get(row.model.as_str())
            && fin.verdict != row.verdict
        {
            any_change = true;
            out.push_str(&format!(
                "- **{}**: {} → {} (agreement {:.2} → {:.2}).\n",
                row.model,
                verdict_text(row.verdict),
                verdict_text(fin.verdict),
                row.agreement,
                fin.agreement,
            ));
        }
    }
    if !any_change {
        out.push_str("No model's calibrated / demote-to-advisory status changes.\n");
    }
    out.push('\n');
    out
}

fn run(args: &Args) -> Result<String, String> {
    let rows = load_outcome_rows(&args.db).map_err(|e| e.to_string())?;
    let labels = load_corpus_labels(&args.labels).map_err(|e| e.to_string())?;
    let (runs, _unlabelled) = join_labelled(&rows, &labels).map_err(|e| e.to_string())?;
    let (reports, skipped) = index_reports(&args.reports_dir)?;
    let recorded_table = reliability_table(&runs);
    let (rederived, changes, missing) = rederive_final_verdicts(&runs, &reports);
    let final_table = reliability_table(&rederived);
    let addendum = render_addendum(
        &args.date,
        &runs,
        &changes,
        missing,
        skipped,
        &recorded_table,
        &final_table,
    );
    if let Some(parent) = args.out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(&args.out, &addendum)
        .map_err(|e| format!("cannot write {}: {e}", args.out.display()))?;
    let mut summary = format!(
        "re-derived {} runs: {} verdicts changed, {} without a report; addendum written to {}\n",
        runs.len(),
        changes.len(),
        missing,
        args.out.display()
    );
    let final_by_model: BTreeMap<&str, &ReliabilityRow> = final_table
        .iter()
        .map(|row| (row.model.as_str(), row))
        .collect();
    for row in &recorded_table {
        if let Some(fin) = final_by_model.get(row.model.as_str()) {
            summary.push_str(&format!(
                "{}: agreement {:.2} -> {:.2}, {} -> {}\n",
                row.model,
                row.agreement,
                fin.agreement,
                verdict_text(row.verdict),
                verdict_text(fin.verdict)
            ));
        }
    }
    Ok(summary)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&argv).and_then(|args| run(&args)) {
        Ok(summary) => {
            print!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("phlow-recalibrate: {error}");
            ExitCode::FAILURE
        }
    }
}
