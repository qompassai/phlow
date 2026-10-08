//! `phlow-trainlab` command-line entry point.
//!
//! Subcommands:
//!
//! - `tasks` — list frozen tasks for a split (JSON).
//! - `eval` — pass@k evaluation over a split.
//! - `run` — one RLOO-statistics experiment run; writes a receipt.
//! - `record-selection` / `open-confirm` — confirmation-gate ledger
//!   operations (see `phlow_trainlab::gate`).
//!
//! `eval` and `run` execute model-generated code through the reward
//! executor and therefore require `--execute-rewards` — the CLI
//! spelling of the operator acknowledgment. Confirmation-split work
//! additionally requires `--ledger` + `--selection-id` and refuses
//! unless the gate shows confirmation open for that selection.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use phlow_trainlab::TrainlabError;
use phlow_trainlab::executor::{Executor, ExecutorConfig};
use phlow_trainlab::gate::ConfirmationGate;
use phlow_trainlab::reward::RewardConfig;
use phlow_trainlab::runner::{RunConfig, evaluate_passk, run_experiment, select_tasks};
use phlow_trainlab::sampler::{OllamaSampler, Sampler, ScriptedSampler};
use phlow_trainlab::task::{Split, family_names};

/// Parsed flags: `--key value` pairs plus boolean `--flag`s.
struct Args {
    values: HashMap<String, String>,
    flags: std::collections::HashSet<String>,
}

impl Args {
    fn parse(raw: &[String]) -> Result<Args, String> {
        let mut values = HashMap::new();
        let mut flags = std::collections::HashSet::new();
        let mut iter = raw.iter().peekable();
        while let Some(arg) = iter.next() {
            let key = arg
                .strip_prefix("--")
                .ok_or_else(|| format!("unexpected argument {arg:?}; options start with --"))?;
            // A value follows iff the next item is not another
            // `--key`; peek so a following key is never consumed.
            let has_value = iter.peek().is_some_and(|next| !next.starts_with("--"));
            if has_value {
                let value = iter.next().expect("peeked value exists");
                values.insert(key.to_string(), value.clone());
            } else {
                flags.insert(key.to_string());
            }
        }
        Ok(Args { values, flags })
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    fn require(&self, key: &str) -> Result<&str, String> {
        self.get(key)
            .ok_or_else(|| format!("missing required option --{key}"))
    }

    fn flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }

    fn parse_or<T: std::str::FromStr>(&self, key: &str, default: T) -> Result<T, String> {
        match self.get(key) {
            Some(raw) => raw
                .parse::<T>()
                .map_err(|_| format!("option --{key} has invalid value {raw:?}")),
            None => Ok(default),
        }
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = raw.split_first() else {
        eprintln!("usage: phlow-trainlab <tasks|eval|run|record-selection|open-confirm> [options]");
        return ExitCode::from(2);
    };
    let args = match Args::parse(rest) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };
    let result = match command.as_str() {
        "tasks" => cmd_tasks(&args),
        "eval" => cmd_eval(&args),
        "run" => cmd_run(&args),
        "record-selection" => cmd_record_selection(&args),
        "open-confirm" => cmd_open_confirm(&args),
        other => Err(format!("unknown subcommand {other:?}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

/// Bridge library errors into CLI messages.
fn lib<T>(result: Result<T, TrainlabError>) -> Result<T, String> {
    result.map_err(|err| err.to_string())
}

fn split_arg(args: &Args) -> Result<Split, String> {
    lib(Split::parse(args.get("split").unwrap_or("dev")))
}

fn families_arg(args: &Args) -> Vec<String> {
    args.get("families")
        .map(|raw| {
            raw.split(',')
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn cmd_tasks(args: &Args) -> Result<(), String> {
    let split = split_arg(args)?;
    let per_family = args.parse_or("per-family", 4_usize)?;
    let families = families_arg(args);
    let tasks = lib(select_tasks(split, per_family, &families))?;
    let rows: Vec<serde_json::Value> = tasks
        .iter()
        .map(|task| {
            serde_json::json!({
                "task_id": task.task_id,
                "family": task.family,
                "entry_point": task.entry_point,
                "prompt": task.prompt,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&rows).map_err(|err| err.to_string())?
    );
    Ok(())
}

/// Build the executor; the `--execute-rewards` flag is mandatory.
fn build_executor(args: &Args) -> Result<Executor, String> {
    if !args.flag("execute-rewards") {
        return Err(
            "refusing to evaluate without --execute-rewards: rewards execute \
             model-generated code with your interpreter (phlow is not a sandbox)"
                .to_string(),
        );
    }
    let deadline_ms = args.parse_or("deadline-ms", 10_000_u64)?;
    lib(Executor::new(ExecutorConfig {
        interpreter_argv: vec![args.get("interpreter").unwrap_or("python3").to_string()],
        deadline: std::time::Duration::from_millis(deadline_ms),
        acknowledge_code_execution: true,
        ..ExecutorConfig::default()
    }))
}

/// Build the sampler from `--sampler stub|ollama`.
fn build_sampler(args: &Args) -> Result<Box<dyn Sampler>, String> {
    match args.get("sampler").unwrap_or("stub") {
        "stub" => {
            let completion = args
                .get("stub-completion")
                .unwrap_or("\n    return x\n")
                .to_string();
            lib(ScriptedSampler::new("stub", vec![completion]))
                .map(|sampler| Box::new(sampler) as Box<dyn Sampler>)
        }
        "ollama" => {
            let model = args.require("model")?;
            let base_url = args
                .get("base-url")
                .unwrap_or("http://127.0.0.1:11434")
                .to_string();
            lib(OllamaSampler::new(
                base_url,
                model,
                args.flag("allow-remote"),
            ))
            .map(|sampler| Box::new(sampler) as Box<dyn Sampler>)
        }
        other => Err(format!(
            "unknown --sampler {other:?} (expected stub|ollama); families: {}",
            family_names().join(", ")
        )),
    }
}

/// Gate check shared by eval/run on the confirmation split.
fn require_gate(args: &Args, split: Split) -> Result<(), String> {
    if split != Split::Confirm {
        return Ok(());
    }
    let ledger = args.require("ledger")?;
    let selection = args.require("selection-id")?;
    let gate = lib(ConfirmationGate::load(ledger))?;
    lib(gate.require_confirmation_open(selection))
}

fn cmd_eval(args: &Args) -> Result<(), String> {
    let split = split_arg(args)?;
    require_gate(args, split)?;
    let per_family = args.parse_or("per-family", 4_usize)?;
    let samples = args.parse_or("samples", 8_usize)?;
    let k = args.parse_or("k", samples)?;
    let temperature = args.parse_or("temperature", 0.35_f64)?;
    let seed = args.parse_or("seed", 8_675_309_u64)?;
    let tasks = lib(select_tasks(split, per_family, &families_arg(args)))?;
    let sampler = build_sampler(args)?;
    let executor = build_executor(args)?;
    let report = lib(evaluate_passk(
        &tasks,
        sampler.as_ref(),
        &executor,
        samples,
        k,
        temperature,
        seed,
    ))?;
    print_json(&report)
}

fn cmd_run(args: &Args) -> Result<(), String> {
    let split = split_arg(args)?;
    require_gate(args, split)?;
    let reward = RewardConfig {
        invalid_penalty: args.parse_or("invalid-penalty", -0.1_f64)?,
        ..RewardConfig::default()
    };
    let config = RunConfig {
        split,
        per_family: args.parse_or("per-family", 4_usize)?,
        groups: args.parse_or("groups", 8_usize)?,
        group_size: args.parse_or("group-size", 4_usize)?,
        temperature: args.parse_or("temperature", 0.35_f64)?,
        seed: args.parse_or("seed", 42_u64)?,
        reward,
        families: families_arg(args),
    };
    let run_id = args
        .get("run-id")
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}-seed{}", split.name(), config.seed));
    let sampler = build_sampler(args)?;
    let executor = build_executor(args)?;
    let receipt = lib(run_experiment(
        &run_id,
        &config,
        sampler.as_ref(),
        &executor,
    ))?;
    let receipt_path = PathBuf::from(args.require("receipt")?);
    lib(receipt.write_new(&receipt_path))?;
    println!(
        "{}",
        serde_json::json!({
            "run_id": receipt.run_id,
            "receipt": receipt_path.display().to_string(),
            "config_sha256": receipt.config_sha256,
            "groups": receipt.groups.len(),
            "samples_total": receipt.samples_total,
            "groups_with_signal": receipt.groups.iter().filter(|group| group.updated).count(),
        })
    );
    Ok(())
}

fn cmd_record_selection(args: &Args) -> Result<(), String> {
    let ledger = args.require("ledger")?;
    let selection = args.require("selection-id")?;
    let mut gate = lib(ConfirmationGate::load(ledger))?;
    lib(gate.record_selection(selection))?;
    print_json(gate.state())
}

fn cmd_open_confirm(args: &Args) -> Result<(), String> {
    let ledger = args.require("ledger")?;
    let selection = args.require("selection-id")?;
    let mut gate = lib(ConfirmationGate::load(ledger))?;
    let record = lib(gate.open_confirmation(selection))?;
    println!(
        "{}",
        serde_json::json!({
            "selection_id": record.selection_id,
            "already_open": record.already_open,
            "state": gate.state(),
        })
    );
    Ok(())
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|err| err.to_string())?
    );
    Ok(())
}
