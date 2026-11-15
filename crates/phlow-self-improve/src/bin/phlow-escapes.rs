//! `phlow-escapes` — record PH-3 escapes and render check
//! proposals from them.
//!
//! Usage:
//!     phlow-escapes record --db PATH --escape-id ID --run-id ID
//!                   --defect TEXT --found-by TEXT --check-name NAME
//!                   --check-argv JSON
//!     phlow-escapes propose --db PATH --out-dir PATH
//!                   --operator NAME
//!                   [--validate NAME=ESCAPED_DIR,FIXED_DIR]...
//!
//! `record` appends one escape, evidence-checked against the run's
//! PH-1 outcome row: the row must exist and its recorded checks
//! must all read `ok`, or the record is refused — a caught defect
//! is not an escape.
//!
//! `propose` clusters the recorded escapes into check proposals,
//! validates each proposal whose `--validate` revision pair was
//! supplied (the check must fail on the escaped revision and pass
//! on the fixed one), writes `check-proposals.md` to the
//! operator-named directory, and queues every gate-passing proposal
//! through `phlow-approval` as a pending record under the named
//! operator allowlist. The queue is in-memory: the printed record
//! ids are this session's queue entries, and the proposal file is
//! the durable artifact. Nothing is admitted, configured, or
//! decided by this tool — admission is the operator's verdict on
//! the queue record plus their own config edit.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use phlow_self_improve::escape::{
    Escape, EscapeStore, ProposalSection, cluster_escapes, queue_check_proposal,
    render_check_proposals, validate_check,
};
use phlow_self_improve::outcome::OutcomeStore;

/// Parsed command line.
enum Command {
    Record {
        db: PathBuf,
        escape: Escape,
    },
    Propose {
        db: PathBuf,
        out_dir: PathBuf,
        operator: String,
        validate: BTreeMap<String, (PathBuf, PathBuf)>,
    },
}

/// Parse argv by hand (bounded, explicit): unknown or missing
/// arguments are usage errors, never guesses.
fn parse_args(argv: &[String]) -> Result<Command, String> {
    let (mode, rest) = argv
        .split_first()
        .ok_or_else(|| "a subcommand is required: record | propose".to_owned())?;
    let mut db: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut operator: Option<String> = None;
    let mut escape_id: Option<String> = None;
    let mut run_id: Option<String> = None;
    let mut defect: Option<String> = None;
    let mut found_by: Option<String> = None;
    let mut check_name: Option<String> = None;
    let mut check_argv: Option<Vec<String>> = None;
    let mut validate: BTreeMap<String, (PathBuf, PathBuf)> = BTreeMap::new();
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
            "--out-dir" => out_dir = Some(PathBuf::from(value(&mut index)?)),
            "--operator" => operator = Some(value(&mut index)?),
            "--escape-id" => escape_id = Some(value(&mut index)?),
            "--run-id" => run_id = Some(value(&mut index)?),
            "--defect" => defect = Some(value(&mut index)?),
            "--found-by" => found_by = Some(value(&mut index)?),
            "--check-name" => check_name = Some(value(&mut index)?),
            "--check-argv" => {
                let raw = value(&mut index)?;
                let parsed: serde_json::Value =
                    serde_json::from_str(&raw).map_err(|e| format!("--check-argv: {e}"))?;
                let items = parsed
                    .as_array()
                    .ok_or_else(|| "--check-argv must be a JSON array of strings".to_owned())?;
                let mut argv = Vec::new();
                for item in items {
                    argv.push(
                        item.as_str()
                            .ok_or_else(|| "--check-argv items must be strings".to_owned())?
                            .to_owned(),
                    );
                }
                check_argv = Some(argv);
            }
            "--validate" => {
                let raw = value(&mut index)?;
                let (name, pair) = raw
                    .split_once('=')
                    .ok_or_else(|| "--validate must be NAME=ESCAPED_DIR,FIXED_DIR".to_owned())?;
                let (escaped, fixed) = pair
                    .split_once(',')
                    .ok_or_else(|| "--validate must be NAME=ESCAPED_DIR,FIXED_DIR".to_owned())?;
                validate.insert(
                    name.to_owned(),
                    (PathBuf::from(escaped), PathBuf::from(fixed)),
                );
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }
    let db = db.ok_or_else(|| "--db is required".to_owned())?;
    match mode.as_str() {
        "record" => Ok(Command::Record {
            db,
            escape: Escape {
                escape_id: escape_id.ok_or_else(|| "--escape-id is required".to_owned())?,
                run_id: run_id.ok_or_else(|| "--run-id is required".to_owned())?,
                defect: defect.ok_or_else(|| "--defect is required".to_owned())?,
                found_by: found_by.ok_or_else(|| "--found-by is required".to_owned())?,
                check_name: check_name.ok_or_else(|| "--check-name is required".to_owned())?,
                check_argv: check_argv.ok_or_else(|| "--check-argv is required".to_owned())?,
            },
        }),
        "propose" => Ok(Command::Propose {
            db,
            out_dir: out_dir.ok_or_else(|| "--out-dir is required".to_owned())?,
            operator: operator.ok_or_else(|| "--operator is required".to_owned())?,
            validate,
        }),
        other => Err(format!("unknown subcommand: {other}")),
    }
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

fn run(command: &Command) -> Result<String, String> {
    match command {
        Command::Record { db, escape } => {
            let outcomes = OutcomeStore::open(db).map_err(|e| e.to_string())?;
            let escapes = EscapeStore::open(db).map_err(|e| e.to_string())?;
            escapes
                .record(&outcomes, escape)
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "recorded": escape.escape_id,
                "run_id": escape.run_id,
                "check_name": escape.check_name,
            })
            .to_string())
        }
        Command::Propose {
            db,
            out_dir,
            operator,
            validate,
        } => {
            let escapes = EscapeStore::open(db).map_err(|e| e.to_string())?;
            let rows = escapes
                .list(phlow_self_improve::escape::ESCAPE_LIST_LIMIT_MAX)
                .map_err(|e| e.to_string())?;
            let clusters = cluster_escapes(&rows);
            let mut queue = phlow_approval::ApprovalQueue::new(&[operator.as_str()])
                .map_err(|e| e.to_string())?;
            let mut sections = Vec::new();
            let mut queued = Vec::new();
            for cluster in clusters {
                let validation = validate
                    .get(&cluster.check_name)
                    .map(|(escaped, fixed)| validate_check(&cluster.check_argv, escaped, fixed));
                let queue_state = match &validation {
                    Some(validation) if validation.gate_holds => {
                        let record = queue_check_proposal(&mut queue, &cluster, validation)
                            .map_err(|e| e.to_string())?;
                        queued.push(serde_json::json!({
                            "check_name": cluster.check_name,
                            "queue_id": record.id,
                            "state": record.state.as_str(),
                        }));
                        Some((record.id, record.state.as_str().to_owned()))
                    }
                    _ => None,
                };
                sections.push(ProposalSection {
                    cluster,
                    validation,
                    queue_state,
                });
            }
            let text = render_check_proposals(&sections);
            std::fs::create_dir_all(out_dir)
                .map_err(|e| format!("create {}: {e}", out_dir.display()))?;
            write_atomic(out_dir, "check-proposals.md", &text)?;
            Ok(serde_json::json!({
                "escapes": rows.len(),
                "proposals": sections.len(),
                "queued": queued,
                "proposal_file": out_dir.join("check-proposals.md").display().to_string(),
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
            eprintln!("phlow-escapes: {error}");
            ExitCode::FAILURE
        }
    }
}
