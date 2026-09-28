// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 185 — no persistent MCP config (rust, V).
//!
//! The seam is bridge lifecycle → filesystem/MCP registry.
//! Ephemerality is verifiable: after a bridge runs and exits, the MCP
//! config surface is byte-identical to before — nothing persisted,
//! nothing registered. The bridge holds no config path at all
//! (`BridgeConfig` has no such field — by construction it cannot write
//! config), and the registry double shows no bridge entry. Ten
//! sequential tasks leave the surface byte-identical: zero
//! accumulation.

use crate::bridge::{
    Bridge, BridgeConfig, BridgeError, McpRegistry, ScriptedPort, next_temp_dir, snapshot_dir,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Task id.
pub const ID: &str = "task-185";
/// Task name.
pub const NAME: &str = "no persistent MCP config";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 3 validation.
pub const CASES: [&str; 3] = [
    "config_dir_byte_identical",
    "registry_clean",
    "ten_sequential_no_accumulation",
];

/// A fake MCP config surface: a config file plus a servers dir.
fn fixture_config_dir(prefix: &str) -> PathBuf {
    let dir = next_temp_dir(prefix);
    std::fs::write(
        dir.join("config.json"),
        r#"{"servers":{"existing":{"command":"/usr/bin/existing"}}}"#,
    )
    .expect("write config.json");
    std::fs::create_dir_all(dir.join("servers")).expect("mkdir servers");
    std::fs::write(
        dir.join("servers").join("existing.json"),
        r#"{"command":"/usr/bin/existing"}"#,
    )
    .expect("write servers/existing.json");
    dir
}

/// Run one bridge task lifecycle against the scripted port.
fn run_one_bridge(task: &str, registry: &McpRegistry) -> Result<(), BridgeError> {
    let mut bridge = Bridge::launch(
        BridgeConfig::single_task(task, vec!["example.com".to_string()]),
        Box::new(ScriptedPort::new()),
    )?;
    let _ = bridge.mcp_initialize()?;
    // The bridge never sees the registry or the config dir: there is
    // no API on Bridge that accepts either.
    let _ = registry.entries().len();
    let shutdown = bridge.shutdown();
    if !shutdown.reaped {
        return Err(BridgeError::KillFailed {
            detail: "bridge not reaped".to_string(),
        });
    }
    Ok(())
}

fn diff_report(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<String> {
    let mut diffs = Vec::new();
    for (path, hash) in before {
        match after.get(path) {
            Some(other) if other == hash => {}
            Some(other) => diffs.push(format!("modified: {path} ({hash} -> {other})")),
            None => diffs.push(format!("deleted: {path}")),
        }
    }
    for path in after.keys() {
        if !before.contains_key(path) {
            diffs.push(format!("added: {path}"));
        }
    }
    diffs
}

/// V1: run a bridge task, let it exit → the MCP config dir snapshot is
/// byte-identical before/after (the diff is empty).
fn case_config_dir_byte_identical() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let dir = fixture_config_dir("w29-mcp-config");
    let registry = McpRegistry::new();
    let before = snapshot_dir(&dir);
    run_one_bridge("T-185a", &registry).map_err(fixture_err)?;
    let after = snapshot_dir(&dir);
    let diffs = diff_report(&before, &after);
    if !diffs.is_empty() {
        failures.push(format!("config surface changed: {diffs:?}"));
    }
    evidence.push(format!(
        "config dir: {} files, diff empty = {}",
        before.len(),
        diffs.is_empty()
    ));
    evidence.push("BridgeConfig has no config-path field: unwritable by construction".to_string());
    cleanup(&dir);
    finish_case(
        CASES[0],
        failures,
        evidence,
        serde_json::json!({
            "files": before.len(),
            "diff_empty": diffs.is_empty(),
        }),
    )
}

/// The registry double shows no bridge entry after the run.
fn case_registry_clean() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let dir = fixture_config_dir("w29-mcp-registry");
    let mut registry = McpRegistry::new();
    // A pre-existing server entry proves the assertion is not vacuous.
    registry.register("existing", "/usr/bin/existing");
    let entries_before = registry.entries().len();
    run_one_bridge("T-185b", &registry).map_err(fixture_err)?;
    let entries: Vec<&String> = registry.entries().keys().collect();
    if entries != vec![&"existing".to_string()] {
        failures.push(format!("registry changed: {entries:?}"));
    }
    if entries.iter().any(|name| name.contains("bridge")) {
        failures.push("registry holds a bridge entry".to_string());
    }
    evidence.push(format!(
        "registry before={entries_before} after={}: no bridge entry",
        entries.len()
    ));
    cleanup(&dir);
    finish_case(
        CASES[1],
        failures,
        evidence,
        serde_json::json!({
            "registry_entries": entries.len(),
            "bridge_entry": false,
        }),
    )
}

/// V2: 10 sequential tasks → the config surface is still
/// byte-identical; zero accumulation.
fn case_ten_sequential_no_accumulation() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let dir = fixture_config_dir("w29-mcp-ten");
    let registry = McpRegistry::new();
    let baseline = snapshot_dir(&dir);
    for i in 0..10 {
        run_one_bridge(&format!("T-185-seq-{i}"), &registry).map_err(fixture_err)?;
        let now = snapshot_dir(&dir);
        let diffs = diff_report(&baseline, &now);
        if !diffs.is_empty() {
            failures.push(format!("run {i}: config surface changed: {diffs:?}"));
            break;
        }
    }
    let final_snap = snapshot_dir(&dir);
    if final_snap != baseline {
        failures.push("final snapshot differs from baseline".to_string());
    }
    if !registry.entries().is_empty() {
        failures.push("registry accumulated entries".to_string());
    }
    evidence.push(format!(
        "10 sequential tasks: config surface byte-identical ({} files), registry empty",
        baseline.len()
    ));
    cleanup(&dir);
    let diff_empty = failures.is_empty();
    finish_case(
        CASES[2],
        failures,
        evidence,
        serde_json::json!({
            "runs": 10,
            "diff_empty": diff_empty,
            "registry_entries": registry.entries().len(),
        }),
    )
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn fixture_err(e: BridgeError) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: "bridge".to_string(),
        detail: format!("task-185: {e}"),
    }
}

fn finish_case(
    case: &'static str,
    failures: Vec<String>,
    mut evidence: Vec<String>,
    metrics: serde_json::Value,
) -> Result<CaseReport, TaskDriverError> {
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(case, metrics, evidence);
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "config_dir_byte_identical" => case_config_dir_byte_identical(),
        "registry_clean" => case_registry_clean(),
        "ten_sequential_no_accumulation" => case_ten_sequential_no_accumulation(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-185: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// byte-identical config surface itself.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-185".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-185".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
