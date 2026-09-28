//! task-16: manifest validation (rust).
//!
//! Drives phlow-experiment's manifest parsers
//! (`manifest::parse_suite_manifest`, `parse_budget_manifest`,
//! `parse_promotion_manifest`, `parse_task_manifest`) through valid
//! manifests and a battery of malformed and adversarial inputs. Every
//! malformed manifest must be rejected with the typed error
//! `ExperimentError::ManifestInvalid`, always naming the file label and
//! the offending key — never a panic, never a coerced value, never a
//! silent acceptance.
//!
//! The parsers validate explicitly against `toml::Value` (no derive
//! macros), reject unknown keys at every level, and bound every array
//! and string. The battery below proves those three properties hold:
//! valid manifests parse with their fields intact, malformed manifests
//! are rejected at the right key, and attacker-shaped inputs (type
//! confusion, oversized arrays, injected keys) are rejected without
//! coercion.

use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_experiment::{
    ExperimentError, MANIFEST_SCHEMA_VERSION, RiskClass, parse_budget_manifest,
    parse_promotion_manifest, parse_suite_manifest, parse_task_manifest,
};

/// Task id.
pub const ID: &str = "task-16";
/// Human-readable name.
pub const NAME: &str = "manifest validation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// The file label every rejection in this battery must carry. The parsers
/// echo the caller's label into `ManifestInvalid.file`, so a wrong label
/// here means the error contract is broken.
const FILE: &str = "task-16";

// ---------------------------------------------------------------------------
// Valid fixtures
// ---------------------------------------------------------------------------

/// Minimal valid suite manifest: two suites.
const VALID_SUITES: &str = r#"schema_version = 1

[[suite]]
id = "unit"
description = "Unit contracts."
required = true

[[suite]]
id = "adversarial"
description = "Adversarial probes."
required = false
"#;

/// Minimal valid budget manifest.
const VALID_BUDGETS: &str = r#"schema_version = 1

[defaults]
workers_max = 4
queue_capacity = 16
children_per_task_max = 8
depth_max = 3
task_deadline_ms = 300000
tool_calls_max = 64
output_bytes_max = 1048576
memory_bytes_max = 1073741824
model_turns_max = 12
changed_files_max = 8
changed_bytes_max = 131072
"#;

/// Minimal valid promotion manifest.
const VALID_PROMOTION: &str = r#"schema_version = 1

[thresholds]
critical_safety_pass_pct = 100
allow_new_failures_on_prior_successes = false
hidden_holdout_gain_required = true
p95_latency_increase_pct_max = 10
cost_increase_pct_max = 20
cost_increase_requires_approval = true
rollback_rehearsal_required = true
"#;

/// Minimal valid task manifest.
const VALID_TASK: &str = r#"schema_version = 1
id = "gauntlet-probe-001"
language = "rust"
kind = "cli"
risk = "normal"
workspace_fixture = "fixtures/rust-cli"
task = "Probe task for manifest validation."

[budget]
wall_ms = 60000
model_turns = 4
tool_calls = 16
changed_files = 0
changed_bytes = 0
workers = 2
queue_depth = 4

[[checks]]
name = "tests"
argv = ["cargo", "test"]
required = true
kind = "test"

[acceptance]
required_files = ["src/main.rs"]
forbidden_paths = ["../secret"]
max_new_dependencies = 0
"#;

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Attempt the task: valid manifests parse; malformed and adversarial
/// manifests are rejected with typed errors naming the file and key.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    let mut evidence = Vec::new();
    match drive(&mut evidence) {
        Ok(()) => TaskOutcome::Pass { evidence },
        Err((where_, how)) => TaskOutcome::Fail {
            where_,
            how,
            evidence,
        },
    }
}

/// The whole battery: valid, malformed, adversarial. Fails on the first
/// deviation: a valid manifest that does not parse, a malformed one that
/// is accepted, or a rejection that is not the typed
/// `ExperimentError::ManifestInvalid` naming the file and key.
fn drive(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    drive_valid(evidence)?;
    drive_malformed(evidence)?;
    drive_adversarial(evidence)?;
    Ok(())
}

/// Phase 1 (validation): the four manifest kinds parse and carry their
/// fields. A rejection of a valid manifest is a driver failure.
fn drive_valid(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    let suites =
        parse_suite_manifest(VALID_SUITES, FILE).map_err(|err| valid_failed("suite", err))?;
    if suites.schema_version != MANIFEST_SCHEMA_VERSION
        || suites.suites.len() != 2
        || suites.suites[0].id != "unit"
        || !suites.suites[0].required
        || suites.suites[1].id != "adversarial"
        || suites.suites[1].required
    {
        return Err(valid_failed("suite", bad_fields("suites")));
    }
    evidence.push("ok: suite manifest parses (2 suites: unit, adversarial)".to_string());

    let budgets =
        parse_budget_manifest(VALID_BUDGETS, FILE).map_err(|err| valid_failed("budget", err))?;
    if budgets.defaults.workers_max != 4
        || budgets.defaults.queue_capacity != 16
        || budgets.defaults.depth_max != 3
        || budgets.defaults.task_deadline_ms != 300_000
    {
        return Err(valid_failed("budget", bad_fields("defaults")));
    }
    evidence.push("ok: budget manifest parses (workers_max=4, depth_max=3)".to_string());

    let promotion = parse_promotion_manifest(VALID_PROMOTION, FILE)
        .map_err(|err| valid_failed("promotion", err))?;
    if promotion.thresholds.critical_safety_pass_pct != 100
        || !promotion.thresholds.cost_increase_requires_approval
        || !promotion.thresholds.hidden_holdout_gain_required
        || promotion.thresholds.allow_new_failures_on_prior_successes
    {
        return Err(valid_failed("promotion", bad_fields("thresholds")));
    }
    evidence.push("ok: promotion manifest parses (critical_safety_pass_pct=100)".to_string());

    let task = parse_task_manifest(VALID_TASK, FILE).map_err(|err| valid_failed("task", err))?;
    if task.id != "gauntlet-probe-001"
        || task.risk != RiskClass::Normal
        || task.budget.wall_ms != 60_000
        || task.checks.len() != 1
        || task.checks[0].name != "tests"
        || task.checks[0].argv != ["cargo", "test"]
        || task.acceptance.required_files != ["src/main.rs"]
    {
        return Err(valid_failed("task", bad_fields("task manifest")));
    }
    evidence.push("ok: task manifest parses (id=gauntlet-probe-001, 1 check)".to_string());
    Ok(())
}

/// A valid manifest that failed to parse, or parsed with wrong fields.
fn valid_failed(label: &str, err: ExperimentError) -> (String, String) {
    (
        "valid-manifest".to_string(),
        format!("valid {label} manifest did not parse as expected: {err}"),
    )
}

/// Synthesize an `ExperimentError` for the "parsed but wrong fields" path
/// so `valid_failed` keeps one shape.
fn bad_fields(what: &str) -> ExperimentError {
    ExperimentError::ManifestInvalid {
        file: FILE,
        key: what.to_string(),
        reason: "parsed but fields did not match the fixture".to_string(),
    }
}

/// Phase 2 (validation): malformed manifests are rejected with the typed
/// error at the right key.
fn drive_malformed(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    drive_malformed_envelope(evidence)?;
    drive_malformed_fields(evidence)?;
    Ok(())
}

/// Envelope breakage: unparseable documents, wrong or missing schema
/// versions, stray top-level keys.
fn drive_malformed_envelope(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    // Not TOML at all: rejected at the document level.
    expect_rejected(
        evidence,
        "garbage toml",
        "schema_version = = =",
        |text, file| parse_suite_manifest(text, file).map(|_| ()),
        "document",
        "",
    )?;
    // Wrong schema version.
    expect_rejected(
        evidence,
        "schema_version = 2",
        &VALID_SUITES.replace("schema_version = 1", "schema_version = 2"),
        |text, file| parse_suite_manifest(text, file).map(|_| ()),
        "schema_version",
        "only schema_version = 1",
    )?;
    // Missing schema_version.
    expect_rejected(
        evidence,
        "missing schema_version",
        "[[suite]]\nid = \"x\"\ndescription = \"d\"\nrequired = true\n",
        |text, file| parse_suite_manifest(text, file).map(|_| ()),
        "manifest.schema_version",
        "is required",
    )?;
    // Unknown top-level key (placed before [defaults] so it is truly top-level).
    expect_rejected(
        evidence,
        "unknown top-level key",
        &VALID_BUDGETS.replace("schema_version = 1\n", "schema_version = 1\nevil = 1\n"),
        |text, file| parse_budget_manifest(text, file).map(|_| ()),
        "manifest.evil",
        "unknown key",
    )?;
    Ok(())
}

/// Field breakage: wrong types and missing required tables/arrays.
fn drive_malformed_fields(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    // Wrong type for a boolean.
    expect_rejected(
        evidence,
        "required as string",
        &VALID_SUITES.replace("required = true", "required = \"yes\""),
        |text, file| parse_suite_manifest(text, file).map(|_| ()),
        "suite[0].required",
        "must be a boolean",
    )?;
    // No suites at all.
    expect_rejected(
        evidence,
        "empty suite manifest",
        "schema_version = 1\n",
        |text, file| parse_suite_manifest(text, file).map(|_| ()),
        "suite",
        "is required",
    )?;
    // Task manifest without its budget table.
    expect_rejected(
        evidence,
        "task manifest without budget",
        &VALID_TASK.replace(
            "[budget]\nwall_ms = 60000\nmodel_turns = 4\ntool_calls = 16\nchanged_files = 0\nchanged_bytes = 0\nworkers = 2\nqueue_depth = 4\n\n",
            "",
        ),
        |text, file| parse_task_manifest(text, file).map(|_| ()),
        "budget",
        "the [budget] table is required",
    )?;
    // Integer threshold given as a string.
    expect_rejected(
        evidence,
        "threshold as string",
        &VALID_PROMOTION.replace(
            "critical_safety_pass_pct = 100",
            "critical_safety_pass_pct = \"100\"",
        ),
        |text, file| parse_promotion_manifest(text, file).map(|_| ()),
        "thresholds.critical_safety_pass_pct",
        "must be an integer",
    )?;
    Ok(())
}

/// Phase 3 (adversarial): attacker-shaped inputs — type confusion,
/// oversized arrays, injected keys — are rejected without coercion.
fn drive_adversarial(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    drive_adversarial_types(evidence)?;
    drive_adversarial_bounds(evidence)?;
    Ok(())
}

/// Type confusion: strings, floats, zeros, and negatives must not coerce
/// into the expected types.
fn drive_adversarial_types(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    // Type confusion: a string schema version must not coerce to 1.
    expect_rejected(
        evidence,
        "schema_version as string",
        &VALID_SUITES.replace("schema_version = 1", "schema_version = \"1\""),
        |text, file| parse_suite_manifest(text, file).map(|_| ()),
        "manifest.schema_version",
        "must be an integer",
    )?;
    // Type confusion: a float budget must not truncate to an integer.
    expect_rejected(
        evidence,
        "workers_max as float",
        &VALID_BUDGETS.replace("workers_max = 4", "workers_max = 4.5"),
        |text, file| parse_budget_manifest(text, file).map(|_| ()),
        "defaults.workers_max",
        "must be an integer",
    )?;
    // Zero where a positive budget is required.
    expect_rejected(
        evidence,
        "workers_max = 0",
        &VALID_BUDGETS.replace("workers_max = 4", "workers_max = 0"),
        |text, file| parse_budget_manifest(text, file).map(|_| ()),
        "defaults.workers_max",
        "must be positive",
    )?;
    // Negative budget.
    expect_rejected(
        evidence,
        "workers_max = -1",
        &VALID_BUDGETS.replace("workers_max = 4", "workers_max = -1"),
        |text, file| parse_budget_manifest(text, file).map(|_| ()),
        "defaults.workers_max",
        "must not be negative",
    )?;
    Ok(())
}

/// Size bombs and smuggled keys: oversized arrays/strings, injected keys,
/// unknown enum spellings, duplicate keys.
fn drive_adversarial_bounds(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    // Oversized array: 17 suites against SUITES_MAX = 16.
    expect_rejected(
        evidence,
        "17 suites (max 16)",
        &suites_with(17),
        |text, file| parse_suite_manifest(text, file).map(|_| ()),
        "suite",
        "at most 16 items allowed",
    )?;
    // Oversized string: 129-char id against the 128-char bound.
    expect_rejected(
        evidence,
        "129-char suite id",
        &VALID_SUITES.replace("id = \"unit\"", &format!("id = \"{}\"", "u".repeat(129))),
        |text, file| parse_suite_manifest(text, file).map(|_| ()),
        "suite[0].id",
        "at most 128 characters",
    )?;
    // Injected unknown key inside a nested table.
    expect_rejected(
        evidence,
        "injected defaults.evil",
        &VALID_BUDGETS.replace("workers_max = 4", "workers_max = 4\nevil = 1"),
        |text, file| parse_budget_manifest(text, file).map(|_| ()),
        "defaults.evil",
        "unknown key",
    )?;
    // Unknown risk class.
    expect_rejected(
        evidence,
        "risk = low",
        &VALID_TASK.replace("risk = \"normal\"", "risk = \"low\""),
        |text, file| parse_task_manifest(text, file).map(|_| ()),
        "risk",
        "unknown risk class",
    )?;
    // Duplicate key: rejected at TOML parse time, still typed.
    expect_rejected(
        evidence,
        "duplicate schema_version",
        "schema_version = 1\nschema_version = 1\n",
        |text, file| parse_suite_manifest(text, file).map(|_| ()),
        "document",
        "",
    )?;
    Ok(())
}

/// Build a suite manifest with `count` entries.
fn suites_with(count: usize) -> String {
    let mut out = String::from("schema_version = 1\n");
    for index in 0..count {
        out.push_str(&format!(
            "\n[[suite]]\nid = \"suite-{index}\"\ndescription = \"Probe suite {index}.\"\nrequired = true\n"
        ));
    }
    out
}

/// Parse `toml` with `parse` and require a typed rejection whose key and
/// reason contain `want_key` / `want_reason`. Acceptance of a malformed
/// manifest, an untyped error, or a rejection that does not name the file
/// and key is a driver failure.
fn expect_rejected(
    evidence: &mut Vec<String>,
    label: &str,
    toml: &str,
    parse: impl Fn(&str, &'static str) -> Result<(), ExperimentError>,
    want_key: &str,
    want_reason: &str,
) -> Result<(), (String, String)> {
    match parse(toml, FILE) {
        Ok(()) => Err((
            "malformed-accepted".to_string(),
            format!("MALFORMED MANIFEST ACCEPTED: {label}"),
        )),
        Err(ExperimentError::ManifestInvalid { file, key, reason }) => {
            if file != FILE {
                return Err((
                    "wrong-file-label".to_string(),
                    format!("{label}: rejection named file {file:?}, want {FILE:?}"),
                ));
            }
            if !key.contains(want_key) || !reason.contains(want_reason) {
                return Err((
                    "wrong-rejection".to_string(),
                    format!(
                        "{label}: rejection key {key:?} / reason {reason:?} \
                         did not match want key {want_key:?} / reason {want_reason:?}"
                    ),
                ));
            }
            if key.is_empty() || reason.is_empty() {
                return Err((
                    "empty-rejection".to_string(),
                    format!("{label}: rejection left the key or reason empty"),
                ));
            }
            evidence.push(format!("reject: {label} -> key {key}: {reason}"));
            Ok(())
        }
        Err(other) => Err((
            "untyped-rejection".to_string(),
            format!("{label}: rejection was not ManifestInvalid: {other}"),
        )),
    }
}
