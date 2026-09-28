// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 198 — JSON is a contract (rust, V).
//!
//! The seam is `phlow status --json` (and the other report commands):
//! the Ghostex machine-output doctrine, adapted to phlow-cli. Valid
//! JSON on every invocation, identical field-name sets across runs,
//! stable entity ids, and schema snapshots checked in so a field
//! rename or a dropped key fails the build instead of the user.
//!
//! Three cases, all validation: read-twice field-set stability plus
//! the `--json`-refused-by-TUI usage error (V1), entity-id stability
//! across runs with a seeded config (V2), and schema snapshots for the
//! `status`, `check`, and `run` report shapes (V3).
//!
//! Required phlow-cli behavior did not exist when this driver was
//! written (no explicit `--json` flag, no refusal for commands that
//! cannot honor it), so the minimum doctrine-conformant change was
//! made in `crates/phlow-cli` and is asserted here — not assumed.

use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::tasks::cli_harness;
use crate::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Task id.
pub const ID: &str = "task-198";
/// Task name.
pub const NAME: &str = "JSON is a contract";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 3 validation.
pub const CASES: [&str; 3] = [
    "read_twice_json_stable",
    "ids_stable_across_runs",
    "schema_snapshots",
];

/// A seed config with named checks: the checks array is the entity-id
/// surface this task asserts stability on. `true` keeps the run
/// hermetic (no network, no model).
const SEED_CONFIG: &str = "[checks.alpha]\ncmd = [\"true\"]\n[checks.beta]\ncmd = [\"true\"]\n";

fn fixture(what: &str, detail: String) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: what.to_string(),
        detail: format!("task-198: {detail}"),
    }
}

fn write_seed_config(dir: &std::path::Path) -> Result<PathBuf, TaskDriverError> {
    let path = dir.join("phlow.toml");
    std::fs::write(&path, SEED_CONFIG)
        .map_err(|e| fixture("seed", format!("cannot write seed config: {e}")))?;
    Ok(path)
}

/// Flag-position parity: `--json` before the subcommand must behave
/// the same as after it, like the other global flags.
fn check_flag_parity(
    dir: &std::path::Path,
    cfg_arg: &str,
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<(), TaskDriverError> {
    let leading = cli_harness::run_phlow_in(dir, &["--json", "-c", cfg_arg, "status"])
        .map_err(|e| fixture("spawn", format!("--json status failed to spawn: {e}")))?;
    if leading.code != Some(0) {
        failures.push(format!("--json status exited {:?}", leading.code));
    }
    cli_harness::parse_json_stdout(&leading, "--json status").map_err(|e| fixture("json", e))?;
    evidence.push("--json accepted before the subcommand (global-flag parity)".to_string());
    Ok(())
}

/// The TUI cannot honor machine output: it must refuse `--json` with
/// a usage error (exit 2), never silently dump human text. Returns
/// the observed exit code for the metrics.
fn check_tui_json_refused(
    failures: &mut Vec<String>,
    evidence: &mut Vec<String>,
) -> Result<Option<i32>, TaskDriverError> {
    let tui = cli_harness::run_phlow(&["tui", "--json"])
        .map_err(|e| fixture("spawn", format!("tui --json failed to spawn: {e}")))?;
    if tui.code != Some(2) {
        failures.push(format!(
            "tui --json exited {:?}, want Some(2) usage error",
            tui.code
        ));
    }
    if tui.stderr.is_empty() {
        failures.push("tui --json printed no usage error on stderr".to_string());
    }
    evidence.push(format!(
        "tui --json: exit {:?}, usage error on stderr",
        tui.code
    ));
    Ok(tui.code)
}

/// V1: run `status --json` twice; both outputs parse as JSON and the
/// field-name sets are identical. Also: `--json` before the subcommand
/// behaves the same (global-flag parity), and `tui --json` is a usage
/// error — the interactive command cannot honor machine output, so it
/// refuses instead of printing human text.
fn case_read_twice_json_stable() -> Result<CaseReport, TaskDriverError> {
    let dir = cli_harness::case_temp_dir("198-v1");
    let cfg = write_seed_config(&dir)?;
    let cfg_arg = cfg.to_string_lossy().to_string();
    let args: &[&str] = &["-c", &cfg_arg, "status", "--json"];
    let first = cli_harness::run_phlow_in(&dir, args)
        .map_err(|e| fixture("spawn", format!("first status --json failed to spawn: {e}")))?;
    let second = cli_harness::run_phlow_in(&dir, args).map_err(|e| {
        fixture(
            "spawn",
            format!("second status --json failed to spawn: {e}"),
        )
    })?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    if first.code != Some(0) {
        failures.push(format!("first status --json exited {:?}", first.code));
    }
    if second.code != Some(0) {
        failures.push(format!("second status --json exited {:?}", second.code));
    }
    let v1 = cli_harness::parse_json_stdout(&first, "first status --json")
        .map_err(|e| fixture("json", e))?;
    let v2 = cli_harness::parse_json_stdout(&second, "second status --json")
        .map_err(|e| fixture("json", e))?;
    let mut paths1 = Vec::new();
    let mut paths2 = Vec::new();
    cli_harness::field_paths(&v1, "", &mut paths1);
    cli_harness::field_paths(&v2, "", &mut paths2);
    paths1.sort();
    paths2.sort();
    paths1.dedup();
    paths2.dedup();
    if paths1 != paths2 {
        failures.push(format!(
            "field-name sets differ across runs: {paths1:?} vs {paths2:?}"
        ));
    }
    evidence.push(format!(
        "status --json: both parse, {} field paths",
        paths1.len()
    ));
    check_flag_parity(&dir, &cfg_arg, &mut failures, &mut evidence)?;
    let tui_code = check_tui_json_refused(&mut failures, &mut evidence)?;
    evidence.push("backend: real phlow binary (subprocess)".to_string());
    let mut report = CaseReport::pass(
        CASES[0],
        serde_json::json!({
            "field_paths": paths1.len(),
            "tui_json_refused_with": tui_code,
            "backend": "real-binary",
        }),
        evidence,
    );
    if !failures.is_empty() {
        report.passed = false;
        report.failures = failures;
    }
    Ok(report)
}

/// Pull the `name` of every check out of a `status` report: the entity
/// ids whose stability V2 asserts.
fn check_names(report: &serde_json::Value) -> Result<Vec<String>, String> {
    report
        .get("checks")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "status report has no checks array".to_string())?
        .iter()
        .map(|c| {
            c.get("name")
                .and_then(|n| n.as_str())
                .map(str::to_string)
                .ok_or_else(|| "a check entry has no string name".to_string())
        })
        .collect()
}

/// V2: the same seeded config, read twice — the entity ids (check
/// names) are identical across runs, and so is the rest of the
/// machine-readable identity surface (`workspace`, `backend`).
fn case_ids_stable_across_runs() -> Result<CaseReport, TaskDriverError> {
    let dir = cli_harness::case_temp_dir("198-v2");
    let cfg = write_seed_config(&dir)?;
    let cfg_arg = cfg.to_string_lossy().to_string();
    let args: &[&str] = &["-c", &cfg_arg, "status", "--json"];
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut names: Vec<Vec<String>> = Vec::new();
    for round in 0..2 {
        let out = cli_harness::run_phlow_in(&dir, args)
            .map_err(|e| fixture("spawn", format!("round {round} failed to spawn: {e}")))?;
        if out.code != Some(0) {
            failures.push(format!("round {round} exited {:?}", out.code));
            continue;
        }
        let report = cli_harness::parse_json_stdout(&out, &format!("round {round}"))
            .map_err(|e| fixture("json", e))?;
        names.push(check_names(&report).map_err(|e| fixture("json", e))?);
        for key in ["workspace", "backend"] {
            let value = report
                .get(key)
                .ok_or_else(|| fixture("json", format!("status report lacks '{key}'")))?;
            evidence.push(format!("round {round} {key}: {value}"));
        }
    }
    if names.len() == 2 && names[0] != names[1] {
        failures.push(format!(
            "entity ids differ across runs: {:?} vs {:?}",
            names[0], names[1]
        ));
    }
    if names.len() == 2 && names[0].is_empty() {
        failures.push("no entity ids observed at all".to_string());
    }
    if let Some(first) = names.first() {
        evidence.push(format!(
            "entity ids stable across runs: {}",
            first.join(", ")
        ));
    }
    evidence.push("backend: real phlow binary (subprocess), seeded config".to_string());
    let mut report = CaseReport::pass(
        CASES[1],
        serde_json::json!({
            "entity_ids": names.first().unwrap_or(&Vec::new()),
            "rounds": names.len(),
            "backend": "real-binary",
        }),
        evidence,
    );
    if !failures.is_empty() {
        report.passed = false;
        report.failures = failures;
    }
    Ok(report)
}

/// Recursive type-only shape of a JSON value: objects keep their
/// field names, arrays keep the first element's shape, scalars keep
/// only their type. A renamed field, a dropped key, or a changed
/// type fails the snapshot comparison.
fn schema_of(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Null => serde_json::json!({"type": "null"}),
        serde_json::Value::Bool(_) => serde_json::json!({"type": "boolean"}),
        serde_json::Value::Number(_) => serde_json::json!({"type": "number"}),
        serde_json::Value::String(_) => serde_json::json!({"type": "string"}),
        serde_json::Value::Array(items) => {
            let item_schema = items
                .first()
                .map(schema_of)
                .unwrap_or(serde_json::json!({"type": "null"}));
            serde_json::json!({"type": "array", "items": item_schema})
        }
        serde_json::Value::Object(map) => {
            let fields: serde_json::Map<String, serde_json::Value> =
                map.iter().map(|(k, v)| (k.clone(), schema_of(v))).collect();
            serde_json::json!({"type": "object", "fields": fields})
        }
    }
}

/// Where the checked-in schema snapshots live.
fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("wave32")
        .join(format!("schema-{name}.json"))
}

/// V3: for `status`, `check`, and `run` with `--json`, compute the
/// type-only schema of the real output and compare it against the
/// checked-in snapshot. Any field rename, dropped key, or type change
/// fails here instead of in a user's parser.
fn case_schema_snapshots() -> Result<CaseReport, TaskDriverError> {
    let dir = cli_harness::case_temp_dir("198-v3");
    let cfg = write_seed_config(&dir)?;
    let cfg_arg = cfg.to_string_lossy().to_string();
    let targets: Vec<(&str, Vec<String>)> = vec![
        (
            "status",
            vec![
                "-c".into(),
                cfg_arg.clone(),
                "status".into(),
                "--json".into(),
            ],
        ),
        (
            "check",
            vec![
                "-c".into(),
                cfg_arg.clone(),
                "check".into(),
                "--json".into(),
            ],
        ),
        // `run` without a backend errors fast (connection refused);
        // the error report has the same stable shape as a success.
        (
            "run",
            vec!["run".into(), "--json".into(), "wave32-probe".into()],
        ),
    ];
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut matched = 0;
    for (name, args) in &targets {
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = cli_harness::run_phlow_in(&dir, &arg_refs)
            .map_err(|e| fixture("spawn", format!("{name} --json failed to spawn: {e}")))?;
        let report = cli_harness::parse_json_stdout(&out, &format!("{name} --json"))
            .map_err(|e| fixture("json", e))?;
        let schema = schema_of(&report);
        let path = snapshot_path(name);
        let snapshot = std::fs::read_to_string(&path)
            .map_err(|e| fixture("snapshot", format!("cannot read {}: {e}", path.display())))?;
        let expected: serde_json::Value = serde_json::from_str(&snapshot).map_err(|e| {
            fixture(
                "snapshot",
                format!("{} is not valid JSON: {e}", path.display()),
            )
        })?;
        if schema == expected {
            matched += 1;
            evidence.push(format!("{name} --json: schema matches {}", path.display()));
        } else {
            failures.push(format!(
                "{name} --json: schema drift vs {} — field renamed, dropped, or retyped",
                path.display()
            ));
        }
    }
    evidence.push("backend: real phlow binary (subprocess)".to_string());
    let mut report = CaseReport::pass(
        CASES[2],
        serde_json::json!({
            "snapshots": targets.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            "matched": matched,
            "backend": "real-binary",
        }),
        evidence,
    );
    if !failures.is_empty() {
        report.passed = false;
        report.failures = failures;
    }
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "read_twice_json_stable" => case_read_twice_json_stable(),
        "ids_stable_across_runs" => case_ids_stable_across_runs(),
        "schema_snapshots" => case_schema_snapshots(),
        _ => Err(fixture("case", format!("unknown case '{case}'"))),
    }
}

/// Task-level entry for the gauntlet runner: the headline case is the
/// read-twice JSON stability case.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-198".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-198".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
