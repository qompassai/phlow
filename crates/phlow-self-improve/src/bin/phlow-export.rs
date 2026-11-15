//! `phlow-export` — export recorded runs to bloch's outcome
//! intake (PH-13).
//!
//! Usage:
//!     phlow-export --db PATH --target PATH --reports DIR --scope SCOPE
//!
//! - `--db` is the feedback store holding the PH-1 outcome rows
//!   and PH-7 captures; `--target` is bloch's outcome store (the
//!   append-only JSONL file bloch serves with `--outcomes`).
//! - `--reports` names a directory of run report files. A report
//!   joins to its run by `outcome_run_id` and supplies the task
//!   text for `asked` — the outcome row stores the plan, not the
//!   task. A run no report joins is refused, never fabricated.
//! - `--scope` is the intake's lane scope: `sfw` refuses every
//!   `nsfw`-labelled record (typed, reported per run);
//!   `unrestricted` exports both lanes — bloch's read side governs
//!   visibility by the labels the records carry. There is no
//!   default: the operator names the intake's scope.
//!
//! The tool appends intake lines to the target and export-ledger
//! rows to the store; it never edits config, prompts, cards, or
//! history. Re-running it is safe: exported runs are reported as
//! already exported and nothing is appended twice (PH-13 dedupe).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use phlow_self_improve::export::{ExportScope, export_outcomes};

/// Most report files one run of the tool will read.
const REPORT_FILES_MAX: usize = 4_096;
/// Largest report file read, in bytes.
const REPORT_BYTES_MAX: u64 = 4 * 1024 * 1024;

/// Parsed command line.
struct Args {
    db: PathBuf,
    target: PathBuf,
    reports: PathBuf,
    scope: ExportScope,
}

/// Parse argv by hand (bounded, explicit): unknown or missing
/// arguments are usage errors, never guesses.
fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut db: Option<PathBuf> = None;
    let mut target: Option<PathBuf> = None;
    let mut reports: Option<PathBuf> = None;
    let mut scope: Option<ExportScope> = None;
    let mut index = 0;
    while index < argv.len() {
        let flag = argv[index].as_str();
        let value = argv
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag {
            "--db" => db = Some(PathBuf::from(value)),
            "--target" => target = Some(PathBuf::from(value)),
            "--reports" => reports = Some(PathBuf::from(value)),
            "--scope" => {
                scope = Some(ExportScope::parse(value).ok_or_else(|| {
                    format!("unknown scope '{value}'; expected sfw or unrestricted")
                })?);
            }
            other => return Err(format!("unknown argument {other}")),
        }
        index += 2;
    }
    Ok(Args {
        db: db.ok_or("missing --db PATH")?,
        target: target.ok_or("missing --target PATH")?,
        reports: reports.ok_or("missing --reports DIR")?,
        scope: scope.ok_or("missing --scope sfw|unrestricted")?,
    })
}

/// The report-directory join: run id → task text, with the counts
/// the operator sees (a report that cannot be read or parsed is
/// counted, never silently skipped).
struct TaskLoad {
    tasks: HashMap<String, String>,
    read: usize,
    joined: usize,
    unreadable: usize,
    duplicates: usize,
}

/// Join run ids to task text from the report files in `dir`.
fn load_tasks(dir: &Path) -> Result<TaskLoad, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot read reports dir {}: {error}", dir.display()))?;
    let mut names: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot list reports dir: {error}"))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            names.push(path);
        }
        if names.len() >= REPORT_FILES_MAX {
            break;
        }
    }
    names.sort();
    let read = names.len();
    let mut tasks: HashMap<String, String> = HashMap::new();
    let mut unreadable = 0usize;
    let mut duplicates = 0usize;
    for path in &names {
        let joined = std::fs::metadata(path)
            .ok()
            .filter(|meta| meta.len() <= REPORT_BYTES_MAX)
            .and_then(|_| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| {
                let run_id = value
                    .get("outcome_run_id")
                    .or_else(|| value.get("run_id"))
                    .and_then(|id| id.as_str())?
                    .to_owned();
                let task = value.get("task").and_then(|task| task.as_str())?.to_owned();
                Some((run_id, task))
            });
        match joined {
            Some((run_id, task)) => {
                if tasks.insert(run_id, task).is_some() {
                    duplicates += 1;
                }
            }
            None => unreadable += 1,
        }
    }
    let joined = tasks.len();
    Ok(TaskLoad {
        tasks,
        read,
        joined,
        unreadable,
        duplicates,
    })
}

fn run(argv: &[String]) -> Result<(), String> {
    let args = parse_args(argv)?;
    let load = load_tasks(&args.reports)?;
    println!(
        "reports: {} read, {} joined to runs, {} unreadable, {} duplicate",
        load.read, load.joined, load.unreadable, load.duplicates
    );
    let summary = export_outcomes(&args.db, &args.target, args.scope, &load.tasks)
        .map_err(|error| error.to_string())?;
    println!(
        "target: {} (scope: {})",
        args.target.display(),
        args.scope.as_str()
    );
    println!("exported: {}", summary.exported.len());
    for run_id in &summary.exported {
        println!("  {run_id}");
    }
    println!("already exported: {}", summary.already_exported.len());
    println!("refused: {}", summary.refused.len());
    for (run_id, reason) in &summary.refused {
        println!("  {run_id}: {reason}");
    }
    Ok(())
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("phlow-export: {error}");
            eprintln!(
                "usage: phlow-export --db PATH --target PATH --reports DIR \
                 --scope sfw|unrestricted"
            );
            ExitCode::FAILURE
        }
    }
}
