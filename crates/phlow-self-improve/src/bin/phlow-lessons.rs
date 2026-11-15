//! `phlow-lessons` — build, inspect, and backfill PH-6 lesson
//! cards from recorded plan divergences.
//!
//! Usage:
//!     phlow-lessons build --db PATH [--class CLASS]
//!     phlow-lessons show --db PATH --class CLASS [--scope LABEL]
//!     phlow-lessons import-report --db PATH --report FILE
//!                   --run-id ID --class CLASS
//!
//! - `build` (re)builds the current lesson card for one class, or
//!   for every class with divergence records. Building over an
//!   unchanged run set appends nothing (the store returns the
//!   current card).
//! - `show` prints the card a retrieval under `--scope` (default
//!   `nsfw`, the label phlow's own runs record) would receive.
//! - `import-report` records one finished run's report file as its
//!   PH-1 outcome row plus its PH-6 divergence record — the
//!   backfill path for runs recorded before PH-6 (e.g. a measurement
//!   corpus), so their evidence can feed cards. It refuses a run id
//!   that already has an outcome row. A report carries no wall
//!   time, so a backfilled row records `wall_ms` 0 (unmeasured) and
//!   the command's output says so: a backfill store is lesson
//!   evidence, not a budget-fitting corpus.
//!
//! The tool writes only store rows (append-only) and stdout. It
//! never edits config, prompts, or cards in place.

use std::path::PathBuf;
use std::process::ExitCode;

use phlow_self_improve::divergence::DivergenceStore;
use phlow_self_improve::lessons::LessonStore;
use phlow_self_improve::outcome::OutcomeStore;
use phlow_self_improve::{RunOutcome, derive_divergence, render_card};

/// Parsed command line.
enum Command {
    Build {
        db: PathBuf,
        class: Option<String>,
    },
    Show {
        db: PathBuf,
        class: String,
        scope: String,
    },
    ImportReport {
        db: PathBuf,
        report: PathBuf,
        run_id: String,
        class: String,
    },
}

/// Parse argv by hand (bounded, explicit): unknown or missing
/// arguments are usage errors, never guesses.
fn parse_args(argv: &[String]) -> Result<Command, String> {
    let (mode, rest) = argv
        .split_first()
        .ok_or_else(|| "a subcommand is required: build | show | import-report".to_owned())?;
    let mut db: Option<PathBuf> = None;
    let mut class: Option<String> = None;
    let mut scope: Option<String> = None;
    let mut report: Option<PathBuf> = None;
    let mut run_id: Option<String> = None;
    let mut index = 0;
    while index < rest.len() {
        let flag = rest[index].as_str();
        let value = |index: &mut usize| -> Result<String, String> {
            *index += 1;
            rest.get(*index)
                .cloned()
                .ok_or_else(|| format!("flag {flag} needs a value"))
        };
        match flag {
            "--db" => db = Some(PathBuf::from(value(&mut index)?)),
            "--class" => class = Some(value(&mut index)?),
            "--scope" => scope = Some(value(&mut index)?),
            "--report" => report = Some(PathBuf::from(value(&mut index)?)),
            "--run-id" => run_id = Some(value(&mut index)?),
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }
    let db = db.ok_or_else(|| "--db is required".to_owned())?;
    match mode.as_str() {
        "build" => Ok(Command::Build { db, class }),
        "show" => Ok(Command::Show {
            db,
            class: class.ok_or_else(|| "--class is required".to_owned())?,
            scope: scope.unwrap_or_else(|| "nsfw".to_owned()),
        }),
        "import-report" => Ok(Command::ImportReport {
            db,
            report: report.ok_or_else(|| "--report is required".to_owned())?,
            run_id: run_id.ok_or_else(|| "--run-id is required".to_owned())?,
            class: class.ok_or_else(|| "--class is required".to_owned())?,
        }),
        other => Err(format!("unknown subcommand: {other}")),
    }
}

fn run(command: &Command) -> Result<String, String> {
    match command {
        Command::Build { db, class } => {
            let divergences = DivergenceStore::open(db).map_err(|e| e.to_string())?;
            let lessons = LessonStore::open(db).map_err(|e| e.to_string())?;
            let classes = match class {
                Some(class) => vec![class.clone()],
                None => divergences.classes().map_err(|e| e.to_string())?,
            };
            let mut out = Vec::new();
            for class in &classes {
                let card = lessons
                    .build_card(&divergences, class)
                    .map_err(|e| e.to_string())?;
                out.push(match card {
                    Some(card) => serde_json::json!({
                        "task_class": card.task_class,
                        "card_version": card.version,
                        "built_over_runs": card.built_over_runs,
                        "content_label": card.content_label,
                    }),
                    None => serde_json::json!({"task_class": class, "card_version": null}),
                });
            }
            Ok(serde_json::Value::Array(out).to_string())
        }
        Command::Show { db, class, scope } => {
            let lessons = LessonStore::open(db).map_err(|e| e.to_string())?;
            match lessons
                .current_card(class, scope)
                .map_err(|e| e.to_string())?
            {
                Some(card) => Ok(render_card(&card)),
                None => Ok(format!(
                    "no lesson card for class \"{class}\" visible under scope \"{scope}\""
                )),
            }
        }
        Command::ImportReport {
            db,
            report,
            run_id,
            class,
        } => {
            let text = std::fs::read_to_string(report)
                .map_err(|e| format!("read {}: {e}", report.display()))?;
            let report_json: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| format!("parse report: {e}"))?;
            let outcome = RunOutcome::from_report(run_id, class, &report_json, 0);
            let outcomes = OutcomeStore::open(db).map_err(|e| e.to_string())?;
            outcomes.record(&outcome).map_err(|e| e.to_string())?;
            let divergence = derive_divergence(
                run_id,
                class,
                &report_json,
                &outcome.origin_lane,
                &outcome.content_label,
            );
            let divergences = DivergenceStore::open(db).map_err(|e| e.to_string())?;
            divergences.record(&divergence).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "run_id": run_id,
                "backfill": true,
                "wall_ms_recorded": 0,
                "task_class": class,
                "planned_steps": divergence.planned_steps,
                "executed_steps": divergence.executed_steps,
                "skipped_steps": divergence.skipped_steps.len(),
                "untracked_steps": divergence.untracked_steps,
                "planned_files_untouched": divergence.planned_files_untouched.len(),
                "added_files": divergence.added_files.len(),
                "repair_cycles": divergence.repair_cycles,
            })
            .to_string())
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse_args(&argv) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("usage error: {error}");
            return ExitCode::from(2);
        }
    };
    match run(&command) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("phlow-lessons: {error}");
            ExitCode::FAILURE
        }
    }
}
