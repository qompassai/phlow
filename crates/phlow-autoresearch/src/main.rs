//! `phlow-autoresearch` CLI: a thin driver over the library.
//!
//! Subcommands:
//! - `run --scenario <file> --worktree <dir> --ledger-dir <dir>
//!   [--iterations <n>]` — replay a scripted scenario (JSON: parallel
//!   `proposals` and `outcomes` arrays) through the real loop and
//!   ledger, printing the run report as JSON. This is the
//!   orchestration exercised end to end with no models attached; the
//!   live proposer/evaluator bindings attach behind the same traits.
//! - `verify --ledger-dir <dir>` — open a ledger, verify its full
//!   hash chain, and print the entry count and head hash.

use std::path::PathBuf;
use std::process::ExitCode;

use phlow_autoresearch::changeset::ChangeSet;
use phlow_autoresearch::clock::SystemClock;
use phlow_autoresearch::error::FailureClass;
use phlow_autoresearch::evaluator::{Evaluation, ScriptedEvaluator, ScriptedOutcome};
use phlow_autoresearch::ledger::Ledger;
use phlow_autoresearch::proposer::{ScriptedProposal, ScriptedProposer};
use phlow_autoresearch::research_loop::{LoopConfig, run_loop};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("phlow-autoresearch: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("run") => cmd_run(&args[1..]),
        Some("verify") => cmd_verify(&args[1..]),
        _ => Err("usage: phlow-autoresearch <run|verify> [flags]".to_string()),
    }
}

/// Fetch the value of `--flag <value>` from an argv slice.
fn flag_value(args: &[String], flag: &str) -> Result<String, String> {
    let position = args
        .iter()
        .position(|arg| arg == flag)
        .ok_or_else(|| format!("missing required flag {flag}"))?;
    args.get(position + 1)
        .cloned()
        .ok_or_else(|| format!("flag {flag} needs a value"))
}

fn cmd_verify(args: &[String]) -> Result<(), String> {
    let ledger_dir = flag_value(args, "--ledger-dir")?;
    let ledger = Ledger::open(&ledger_dir).map_err(|err| err.to_string())?;
    let summary = serde_json::json!({
        "entries": ledger.len(),
        "head_sha256": ledger.head_sha256(),
        "incumbent_metric": ledger.incumbent_metric(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).map_err(|err| err.to_string())?
    );
    Ok(())
}

fn cmd_run(args: &[String]) -> Result<(), String> {
    let scenario_path = flag_value(args, "--scenario")?;
    let worktree = PathBuf::from(flag_value(args, "--worktree")?);
    let ledger_dir = PathBuf::from(flag_value(args, "--ledger-dir")?);
    let text = std::fs::read_to_string(&scenario_path).map_err(|err| err.to_string())?;
    let scenario: serde_json::Value = serde_json::from_str(&text).map_err(|err| err.to_string())?;
    let (proposals, outcomes) = parse_scenario(&scenario)?;

    let mut config = LoopConfig::new(worktree, ledger_dir.clone());
    if let Ok(raw) = flag_value(args, "--iterations") {
        config.iterations_max = raw
            .parse()
            .map_err(|_| format!("--iterations value {raw:?} is not a number"))?;
    }

    let mut proposer = ScriptedProposer::new(proposals);
    let mut evaluator = ScriptedEvaluator::new(outcomes);
    let mut ledger = Ledger::open(&ledger_dir).map_err(|err| err.to_string())?;
    let clock = SystemClock::new();
    let report = run_loop(&config, &mut proposer, &mut evaluator, &mut ledger, &clock)
        .map_err(|err| err.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|err| err.to_string())?
    );
    Ok(())
}

/// Parse the scenario document into scripted proposals and outcomes.
/// The shape is deliberately flat JSON so scenarios are easy to write
/// and diff: `proposals` items are `{"change_set": {...}}` or
/// `{"invalid": "message"}`; `outcomes` items are `{"metric": 0.5}`,
/// `{"evaluation": {...}}`, `{"fail": "message"}`, or
/// `{"advance_ms": 1000, "metric": 0.5}`.
fn parse_scenario(
    scenario: &serde_json::Value,
) -> Result<(Vec<ScriptedProposal>, Vec<ScriptedOutcome>), String> {
    let proposals_json = scenario
        .get("proposals")
        .and_then(serde_json::Value::as_array)
        .ok_or("scenario.proposals must be an array")?;
    let outcomes_json = scenario
        .get("outcomes")
        .and_then(serde_json::Value::as_array)
        .ok_or("scenario.outcomes must be an array")?;

    let mut proposals = Vec::new();
    for item in proposals_json {
        if let Some(change_set) = item.get("change_set") {
            let change_set: ChangeSet =
                serde_json::from_value(change_set.clone()).map_err(|err| err.to_string())?;
            proposals.push(ScriptedProposal::ChangeSet(change_set));
        } else if let Some(message) = item.get("invalid").and_then(serde_json::Value::as_str) {
            proposals.push(ScriptedProposal::Invalid(message.to_string()));
        } else {
            return Err(format!("unrecognized proposal entry: {item}"));
        }
    }

    let mut outcomes = Vec::new();
    for item in outcomes_json {
        if let Some(evaluation) = item.get("evaluation") {
            let evaluation: Evaluation =
                serde_json::from_value(evaluation.clone()).map_err(|err| err.to_string())?;
            outcomes.push(ScriptedOutcome::Evaluation(evaluation));
        } else if let Some(message) = item.get("fail").and_then(serde_json::Value::as_str) {
            outcomes.push(ScriptedOutcome::Fail(
                FailureClass::EvaluationFailed,
                message.to_string(),
            ));
        } else if let Some(metric) = item.get("metric").and_then(serde_json::Value::as_f64) {
            if let Some(advance_ms) = item.get("advance_ms").and_then(serde_json::Value::as_u64) {
                outcomes.push(ScriptedOutcome::AdvanceThenMetric { advance_ms, metric });
            } else {
                outcomes.push(ScriptedOutcome::Evaluation(Evaluation::metric_only(
                    metric,
                    "scenario metric",
                )));
            }
        } else {
            return Err(format!("unrecognized outcome entry: {item}"));
        }
    }
    Ok((proposals, outcomes))
}
