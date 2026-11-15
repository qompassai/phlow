//! `phlow-propose` — render the PH-2 budget proposal and the PH-5
//! reviewer-calibration report from recorded outcomes.
//!
//! Usage:
//!     phlow-propose --db PATH --labels PATH --out-dir PATH
//!                   [--margin-num N] [--margin-den D]
//!
//! The tool reads the PH-1 outcome store **read-only** and the
//! corpus manifest, fits budgets on the fit split, replays them
//! against the held-out split, and — only if the PH-2 gate holds
//! (no held-out success cut) — writes two proposal files to the
//! operator-named directory:
//!
//! - `budget-proposal.md` (PH-2)
//! - `reviewer-calibration.md` (PH-5)
//!
//! It writes nothing else: no config, no budget, no store row. A
//! validation refusal exits non-zero and writes nothing at all.

use std::path::PathBuf;
use std::process::ExitCode;

use phlow_self_improve::budget::{
    Margin, fit_budgets, join_labelled, load_corpus_labels, load_outcome_rows,
    render_budget_proposal, replay_all, validate_budgets,
};
use phlow_self_improve::reviewer_cal::{reliability_table, render_calibration_report};

/// Parsed command line.
struct Args {
    db: PathBuf,
    labels: PathBuf,
    out_dir: PathBuf,
    margin: Margin,
}

/// Parse argv by hand (bounded, explicit): unknown or missing
/// arguments are usage errors, never guesses.
fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut db: Option<PathBuf> = None;
    let mut labels: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut margin_num: u64 = Margin::DEFAULT.num;
    let mut margin_den: u64 = Margin::DEFAULT.den;
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
            "--out-dir" => out_dir = Some(PathBuf::from(value(&mut index)?)),
            "--margin-num" => {
                margin_num = value(&mut index)?
                    .parse()
                    .map_err(|_| "--margin-num must be an unsigned integer".to_owned())?;
            }
            "--margin-den" => {
                margin_den = value(&mut index)?
                    .parse()
                    .map_err(|_| "--margin-den must be an unsigned integer".to_owned())?;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }
    let margin = Margin::new(margin_num, margin_den).map_err(|e| e.to_string())?;
    Ok(Args {
        db: db.ok_or_else(|| "--db is required".to_owned())?,
        labels: labels.ok_or_else(|| "--labels is required".to_owned())?,
        out_dir: out_dir.ok_or_else(|| "--out-dir is required".to_owned())?,
        margin,
    })
}

/// Write one file atomically: temp file in the same directory, then
/// rename over the target, so a crash never leaves a torn proposal.
fn write_atomic(dir: &std::path::Path, name: &str, text: &str) -> Result<(), String> {
    let temp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&temp, text).map_err(|e| format!("write {}: {e}", temp.display()))?;
    let target = dir.join(name);
    std::fs::rename(&temp, &target).map_err(|e| format!("rename to {}: {e}", target.display()))?;
    Ok(())
}

fn run(args: &Args) -> Result<String, String> {
    let rows = load_outcome_rows(&args.db).map_err(|e| e.to_string())?;
    let labels = load_corpus_labels(&args.labels).map_err(|e| e.to_string())?;
    let (runs, unlabelled) = join_labelled(&rows, &labels).map_err(|e| e.to_string())?;
    let fits = fit_budgets(&runs, args.margin);
    // The PH-2 gate is enforced before anything is written.
    validate_budgets(&runs, &fits).map_err(|e| e.to_string())?;
    let (per_class, total) = replay_all(&runs, &fits);
    let table = reliability_table(&runs);
    let unattributed = runs
        .iter()
        .filter(|run| run.label.reviewer_model.is_none())
        .count();
    let budget_text = render_budget_proposal(&fits, &per_class, &total, args.margin, unlabelled);
    let calibration_text = render_calibration_report(&table, unattributed);
    std::fs::create_dir_all(&args.out_dir)
        .map_err(|e| format!("create {}: {e}", args.out_dir.display()))?;
    write_atomic(&args.out_dir, "budget-proposal.md", &budget_text)?;
    write_atomic(&args.out_dir, "reviewer-calibration.md", &calibration_text)?;
    let summary = serde_json::json!({
        "rows_loaded": rows.len(),
        "labelled_runs": runs.len(),
        "unlabelled_rows": unlabelled,
        "classes": fits.iter().map(|fit| serde_json::json!({
            "task_class": fit.task_class,
            "proposed": fit.proposal.is_some(),
            "escalated": fit.proposal.as_ref().is_some_and(|p| p.escalated),
        })).collect::<Vec<_>>(),
        "held_out_successes_cut": total.held_out_successes_cut,
        "wasted_turn_reduction_percent": total.wasted_turn_reduction_percent(),
        "demotions_proposed": table.iter()
            .filter(|row| row.verdict
                == phlow_self_improve::reviewer_cal::CalibrationVerdict::DemoteToAdvisory)
            .map(|row| row.model.clone())
            .collect::<Vec<_>>(),
    });
    Ok(summary.to_string())
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("usage error: {error}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("phlow-propose: {error}");
            ExitCode::FAILURE
        }
    }
}
