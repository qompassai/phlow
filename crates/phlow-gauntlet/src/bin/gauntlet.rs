#![forbid(unsafe_code)]

//! `gauntlet` — CLI driver for the 130-task orchestration gauntlet (130 implemented).
//!
//! ```sh
//! gauntlet list
//! gauntlet run <task-id> [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]
//! gauntlet run-all [--report-dir DIR] [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]
//! gauntlet verify-claims <ledger.json>
//! ```
//!
//! Argument parsing is manual and bounded: four subcommands, four flags, no
//! external CLI framework. Unknown flags are rejected, never ignored.

use phlow_gauntlet::{Ctx, GauntletError, TaskOutcome, TaskReport, bound_evidence, tasks};
use serde_json::{Map, Value};
use std::path::PathBuf;
use std::process::ExitCode;

/// Usage text printed on bad invocation.
const USAGE: &str = "usage:\n  gauntlet list\n  gauntlet run <task-id> [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]\n  gauntlet run-all [--report-dir DIR] [--nvim-bin PATH] [--diver-lua PATH] [--work-dir PATH]\n  gauntlet verify-claims <ledger.json>";

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
    let mut unwired = 0;
    for id in tasks::TASK_IDS {
        // Loud, never silent: a listed-but-unwired id is exactly how the
        // wave-28 summary shipped fiction. task_meta alone would hide it.
        if tasks::is_wired(id) {
            if let Some((name, kind)) = tasks::task_meta(id) {
                println!("{id}\t{kind}\t{name}");
            }
        } else {
            println!("{id}\tUNWIRED");
            unwired += 1;
        }
    }
    if unwired > 0 {
        eprintln!("gauntlet: {unwired} listed task(s) have no dispatch/metadata arms");
        return ExitCode::from(1);
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

/// The ledger names the exact code commit the producer's claims were
/// written against. The verifier's checkout must contain that commit
/// (ancestor check, not equality: the ledger itself lives in a child
/// commit, since a commit hash covers the ledger and can never name its
/// own tree). Fail closed on any git failure.
fn ledger_commit_ok(ledger: &Value) -> Result<(), String> {
    let Some(commit) = ledger.get("commit").and_then(Value::as_str) else {
        return Ok(());
    };
    if commit.is_empty() {
        return Err("ledger 'commit' is empty".to_string());
    }
    let status = std::process::Command::new("git")
        .args(["merge-base", "--is-ancestor", commit, "HEAD"])
        .status()
        .map_err(|e| format!("git unavailable ({e}); cannot bind ledger to a commit"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "ledger names commit '{commit}' which is not an ancestor of HEAD: \
             check out the claimed tree first"
        ))
    }
}

/// Gate zero: a wave's claim ledger is checked against the wiring tables
/// BEFORE any expensive gate runs.
///
/// Ledger shape:
///   {"wave": "wave-32", "commit": "<sha>", "claims": ["task-197", ...]}
///
/// Every claim must name a real task id AND be wired in both dispatch and
/// metadata. Exit 0 = all verified, 1 = claim rejected, 2 = bad invocation.
fn cmd_verify_claims(path: &str) -> ExitCode {
    const CLAIMS_MAX: usize = 1024;

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("gauntlet: cannot read ledger '{path}': {e}");
            return ExitCode::from(2);
        }
    };
    let ledger: Value = match serde_json::from_str(&text) {
        Ok(ledger) => ledger,
        Err(e) => {
            eprintln!("gauntlet: ledger is not valid JSON: {e}");
            return ExitCode::from(2);
        }
    };
    if let Err(e) = ledger_commit_ok(&ledger) {
        eprintln!("gauntlet: {e}");
        return ExitCode::from(1);
    }
    let claims = match ledger.get("claims").and_then(Value::as_array) {
        Some(claims) => claims,
        None => {
            eprintln!("gauntlet: ledger needs a 'claims' array of task ids");
            return ExitCode::from(2);
        }
    };
    if claims.is_empty() || claims.len() > CLAIMS_MAX {
        eprintln!("gauntlet: ledger must claim between 1 and {CLAIMS_MAX} tasks");
        return ExitCode::from(2);
    }
    let mut seen = std::collections::HashSet::with_capacity(claims.len());
    for claim in claims {
        let id = match claim.as_str() {
            Some(id) => id,
            None => {
                eprintln!("gauntlet: claim is not a string: {claim}");
                return ExitCode::from(2);
            }
        };
        if !seen.insert(id) {
            eprintln!("gauntlet: duplicate claim '{id}'");
            return ExitCode::from(2);
        }
        if !tasks::TASK_IDS.contains(&id) {
            eprintln!("gauntlet: unknown task id '{id}'");
            return ExitCode::from(1);
        }
        if !tasks::is_wired(id) {
            eprintln!("gauntlet: CLAIMED BUT NOT WIRED: '{id}'");
            return ExitCode::from(1);
        }
    }
    println!(
        "gauntlet: {} claim(s) verified: every claimed task is wired",
        claims.len()
    );
    ExitCode::SUCCESS
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
        "verify-claims" => {
            let Some((path, flags)) = rest.split_first() else {
                eprintln!("gauntlet verify-claims needs a ledger path\n{USAGE}");
                return ExitCode::from(2);
            };
            if !flags.is_empty() {
                eprintln!("gauntlet verify-claims takes no flags\n{USAGE}");
                return ExitCode::from(2);
            }
            cmd_verify_claims(path)
        }
        other => {
            eprintln!("gauntlet: unknown subcommand '{other}'\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
