#![forbid(unsafe_code)]

//! `gauntlet` — CLI driver for the 115-task orchestration gauntlet (105 implemented).
//!
//! ```sh
//! gauntlet list
//! gauntlet run <task-id> [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]
//! gauntlet run-all [--report-dir DIR] [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]
//! ```
//!
//! Argument parsing is manual and bounded: three subcommands, four flags, no
//! external CLI framework. Unknown flags are rejected, never ignored.

use phlow_gauntlet::{Ctx, GauntletError, TaskOutcome, TaskReport, bound_evidence, tasks};
use serde_json::{Map, Value};
use std::path::PathBuf;
use std::process::ExitCode;

/// Usage text printed on bad invocation.
const USAGE: &str = "usage:\n  gauntlet list\n  gauntlet run <task-id> [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]\n  gauntlet run-all [--report-dir DIR] [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]";

/// Parsed CLI options. All paths are required for `run`/`run-all`: the
/// gauntlet never guesses where Matt's editor or config live.
struct Opts {
    nvim_bin: Option<PathBuf>,
    diver_lua: Option<PathBuf>,
    work_dir: Option<PathBuf>,
    report_dir: Option<PathBuf>,
}

fn parse_flags(args: &[String]) -> Result<Opts, String> {
    let mut opts = Opts {
        nvim_bin: None,
        diver_lua: None,
        work_dir: None,
        report_dir: None,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--nvim-bin" => {
                i += 1;
                opts.nvim_bin = Some(flag_value(args, i, "--nvim-bin")?);
            }
            "--diver-lua" => {
                i += 1;
                opts.diver_lua = Some(flag_value(args, i, "--diver-lua")?);
            }
            "--work-dir" => {
                i += 1;
                opts.work_dir = Some(flag_value(args, i, "--work-dir")?);
            }
            "--report-dir" => {
                i += 1;
                opts.report_dir = Some(flag_value(args, i, "--report-dir")?);
            }
            other => return Err(format!("unknown flag '{other}'\n{USAGE}")),
        }
        i += 1;
    }
    Ok(opts)
}

fn flag_value(args: &[String], i: usize, flag: &str) -> Result<PathBuf, String> {
    args.get(i)
        .map(PathBuf::from)
        .ok_or_else(|| format!("flag '{flag}' needs a value\n{USAGE}"))
}

fn build_ctx(opts: &Opts) -> Result<Ctx, GauntletError> {
    let nvim_bin = opts
        .nvim_bin
        .clone()
        .ok_or(GauntletError::EmptyField { field: "nvim_bin" })?;
    let diver_lua = opts.diver_lua.clone().ok_or(GauntletError::EmptyField {
        field: "diver_lua_dir",
    })?;
    let work_dir = opts
        .work_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("phlow-gauntlet"));
    Ctx::new(nvim_bin, diver_lua, work_dir)
}

/// Render one report as a JSON object. Built from a `Map` so the shape is
/// fixed and every string is escaped by `serde_json`, not by hand.
fn report_json(r: &TaskReport) -> String {
    let mut m = Map::new();
    m.insert("id".to_string(), Value::from(r.id));
    m.insert("name".to_string(), Value::from(r.name));
    m.insert("kind".to_string(), Value::from(r.kind.to_string()));
    m.insert("duration_ms".to_string(), Value::from(r.duration_ms));
    match &r.outcome {
        TaskOutcome::Pass { evidence } => {
            m.insert("outcome".to_string(), Value::from("pass"));
            m.insert(
                "evidence".to_string(),
                Value::from(bound_evidence(evidence.clone())),
            );
        }
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            m.insert("outcome".to_string(), Value::from("fail"));
            m.insert("where".to_string(), Value::from(where_.clone()));
            m.insert("how".to_string(), Value::from(how.clone()));
            m.insert(
                "evidence".to_string(),
                Value::from(bound_evidence(evidence.clone())),
            );
        }
    }
    Value::Object(m).to_string()
}

fn cmd_list() -> ExitCode {
    for id in tasks::TASK_IDS {
        if let Some((name, kind)) = tasks::task_meta(id) {
            println!("{id}\t{kind}\t{name}");
        }
    }
    ExitCode::SUCCESS
}

fn cmd_run(id: &str, opts: &Opts) -> ExitCode {
    let ctx = match build_ctx(opts) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("gauntlet: {e}");
            return ExitCode::from(2);
        }
    };
    match tasks::run_task(id, &ctx) {
        Ok(report) => {
            let passed = report.passed();
            println!("{}", report_json(&report));
            if passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("gauntlet: {e}");
            ExitCode::from(2)
        }
    }
}

fn cmd_run_all(opts: &Opts) -> ExitCode {
    let ctx = match build_ctx(opts) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("gauntlet: {e}");
            return ExitCode::from(2);
        }
    };
    let mut reports: Vec<String> = Vec::new();
    let mut failed: Vec<&str> = Vec::new();
    for id in tasks::TASK_IDS {
        match tasks::run_task(id, &ctx) {
            Ok(report) => {
                if !report.passed() {
                    failed.push(id);
                }
                // One JSON object per line: streaming, bounded memory.
                println!("{}", report_json(&report));
                reports.push(report_json(&report));
            }
            Err(e) => {
                eprintln!("gauntlet: task {id}: {e}");
                failed.push(id);
            }
        }
    }
    let failed_json = Value::from(failed.clone()).to_string();
    let summary = format!(
        "{{\"tasks\":[{}],\"failed\":{failed_json}}}",
        reports.join(",")
    );
    if let Some(dir) = &opts.report_dir {
        let path = dir.join("gauntlet-report.json");
        if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, &summary)) {
            eprintln!("gauntlet: cannot write report: {e}");
            return ExitCode::from(2);
        }
        eprintln!("gauntlet: report written to {}", path.display());
    }
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        eprintln!("gauntlet: {} task(s) failed", failed.len());
        ExitCode::from(1)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((sub, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match sub.as_str() {
        "list" => {
            if !rest.is_empty() {
                eprintln!("gauntlet list takes no flags\n{USAGE}");
                return ExitCode::from(2);
            }
            cmd_list()
        }
        "run" => {
            let Some((id, flags)) = rest.split_first() else {
                eprintln!("gauntlet run needs a task id\n{USAGE}");
                return ExitCode::from(2);
            };
            match parse_flags(flags) {
                Ok(opts) => cmd_run(id, &opts),
                Err(e) => {
                    eprintln!("gauntlet: {e}");
                    ExitCode::from(2)
                }
            }
        }
        "run-all" => match parse_flags(rest) {
            Ok(opts) => cmd_run_all(&opts),
            Err(e) => {
                eprintln!("gauntlet: {e}");
                ExitCode::from(2)
            }
        },
        other => {
            eprintln!("gauntlet: unknown subcommand '{other}'\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
