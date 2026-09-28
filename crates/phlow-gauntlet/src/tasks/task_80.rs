//! task-80: local model selection tradeoffs (nvim-lua driver + harness).
//!
//! The design asks for the local model selector: per tool, choose
//! among local models on latency, memory, and quality tradeoffs,
//! with model cards and measured benchmarks as inputs, trading
//! quality for speed with explicit rationale.
//!
//! Seam mapping (verified, not invented): diver's
//! `ai.harness.adapter.negotiate(adapters, needs)` selects the FIRST
//! adapter in sorted name order whose probed boolean capabilities
//! satisfy every requested need. The negotiated vocabulary is
//! exactly the seven boolean `CAPABILITY_KEYS` in
//! `ai.harness.types` (streaming, cancellation, resume, permissions,
//! artifacts, remote, tools); `max_input_bytes` is an optional
//! integer the negotiation never reads. There is no model registry,
//! no model card, no latency/memory/quality signal, no benchmark, no
//! rationale — the function returns the adapter table, full stop.
//!
//! Four cases: two validation, two adversarial — two run as nvim-lua
//! driver probes against the REAL adapter module (machine-readable
//! traces land in the task work dir), two as harness probes over
//! those traces. The task-level verdict is `fail` at `"seam"`.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//! The design's model selector is a different component than diver's
//! adapter negotiator; whether diver should gain one is Matt's call.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence, run_nvim_lua_driver_with_env};
use std::fmt;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-80";
/// Human-readable name.
pub const NAME: &str = "local model selection tradeoffs";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Probe cases the driver runs, in order:
/// two validation (driver probes), two adversarial (harness probes).
pub const CASES: [&str; 4] = [
    "selection_strategy_reality",
    "model_selection_absent",
    "capability_needs_boolean_only",
    "no_tradeoff_record",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-80 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// A driver probe failed.
    Probe {
        /// Which probe.
        case: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-80: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-80: probe {case} failed: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

fn fixture_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Fixture {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Case verdicts
// ---------------------------------------------------------------------------

/// The parsed verdict of one case.
#[derive(Debug, Clone)]
pub struct CaseReport {
    /// Which case ran.
    pub case: String,
    /// Whether the case's own assertions held.
    pub passed: bool,
    /// Measured numbers.
    pub metrics: serde_json::Value,
    /// Diagnostic lines from the case.
    pub evidence: Vec<String>,
    /// Failing assertion details, empty when `passed`.
    pub failures: Vec<String>,
}

impl CaseReport {
    fn pass(case: &'static str, metrics: serde_json::Value, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: true,
            metrics,
            evidence,
            failures: Vec::new(),
        }
    }

    fn fail(case: &'static str, failure: String, evidence: Vec<String>) -> Self {
        Self {
            case: case.to_string(),
            passed: false,
            metrics: serde_json::json!({}),
            evidence,
            failures: vec![failure],
        }
    }
}

// ---------------------------------------------------------------------------
// Driver probes
// ---------------------------------------------------------------------------

/// Task work dir: the nvim runner creates `ctx.work_dir/task-80`.
fn work_dir(ctx: &Ctx) -> std::path::PathBuf {
    ctx.work_dir.join("task-80")
}

/// Run one driver scenario and convert its [`TaskOutcome`] into a case
/// report.
fn run_driver_scenario(ctx: &Ctx, case: &'static str, scenario: &str) -> CaseReport {
    match run_nvim_lua_driver_with_env(
        ctx,
        "task_80.lua",
        "task-80",
        &[("GAUNTLET_SCENARIO", scenario)],
    ) {
        TaskOutcome::Pass { evidence } => {
            CaseReport::pass(case, serde_json::json!({"scenario": scenario}), evidence)
        }
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => CaseReport::fail(
            case,
            format!("driver scenario {scenario} failed at {where_}: {how}"),
            evidence,
        ),
    }
}

/// Read a machine-readable trace the driver wrote into the work dir.
fn read_trace(ctx: &Ctx, name: &str) -> Result<serde_json::Value, DriverError> {
    let path = work_dir(ctx).join(name);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| fixture_error(&format!("trace {name}"), format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|e| fixture_error(&format!("trace {name}"), format!("unparseable: {e}")))
}

// ---------------------------------------------------------------------------
// Harness probes
// ---------------------------------------------------------------------------

/// A1: the negotiation contract is boolean-only. Over the driver's
/// selection trace: every `needs` table holds only booleans, and
/// every adapter's probed capabilities hold only the seven boolean
/// keys — no latency, memory, or quality fields anywhere.
fn case_capability_needs_boolean_only(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "capability_needs_boolean_only";
    let mut evidence = Vec::new();
    let trace = read_trace(ctx, "selection-trace.json")?;
    let probes = trace
        .get("probes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| fixture_error("selection trace", "missing probes array"))?;
    for (i, probe) in probes.iter().enumerate() {
        // An empty Lua table encodes as `[]`; treat an empty array as
        // empty needs.
        let needs_value = probe
            .get("needs")
            .ok_or_else(|| fixture_error("selection trace", format!("probe {i} missing needs")))?;
        let needs_empty_array = needs_value.as_array().is_some_and(|a| a.is_empty());
        let needs = if needs_empty_array {
            None
        } else {
            Some(needs_value.as_object().ok_or_else(|| {
                fixture_error("selection trace", format!("probe {i} needs not an object"))
            })?)
        };
        if let Some(needs) = needs {
            for (key, value) in needs {
                if !value.is_boolean() {
                    return Ok(CaseReport::fail(
                        CASE,
                        format!("probe {i} need {key} is not boolean: {value}"),
                        evidence,
                    ));
                }
            }
        }
        evidence.push(format!(
            "probe {i} needs are all boolean: {}",
            probe.get("label").unwrap_or(&serde_json::Value::Null)
        ));
    }
    let adapters = trace
        .get("adapters")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| fixture_error("selection trace", "missing adapters object"))?;
    for (name, adapter) in adapters {
        let caps = adapter
            .get("caps")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                fixture_error("selection trace", format!("adapter {name} missing caps"))
            })?;
        for (key, value) in caps {
            if !value.is_boolean() {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("adapter {name} capability {key} is not boolean: {value}"),
                    evidence,
                ));
            }
            let key = key.to_lowercase();
            for banned in ["latency", "memory", "quality", "benchmark"] {
                if key.contains(banned) {
                    return Ok(CaseReport::fail(
                        CASE,
                        format!("adapter {name} exposes tradeoff capability {key}"),
                        evidence,
                    ));
                }
            }
        }
        evidence.push(format!(
            "adapter {name}: {} boolean capability keys, no latency/memory/quality/benchmark fields",
            caps.len()
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"probes": probes.len(), "adapters": adapters.len()}),
        evidence,
    ))
}

/// A2: no tradeoff record exists. The model-selection scan finds zero
/// tradeoff-vocabulary hits in the real module source; the only
/// `model` hits are telemetry event-kind strings; and the selection
/// trace records selected adapter names only — no rationale, no
/// quality, no latency, no memory per decision.
fn case_no_tradeoff_record(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_tradeoff_record";
    let mut evidence = Vec::new();
    let scan = read_trace(ctx, "modelscan.json")?;
    let tradeoff_hits = scan
        .get("tradeoff_hits")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| fixture_error("model scan", "missing tradeoff_hits array"))?;
    evidence.push(format!(
        "tradeoff-vocabulary scan over adapter.lua + types.lua: {} hit(s)",
        tradeoff_hits.len()
    ));
    if !tradeoff_hits.is_empty() {
        let first = &tradeoff_hits[0];
        return Ok(CaseReport::fail(
            CASE,
            format!("tradeoff vocabulary present in module source: {first}"),
            evidence,
        ));
    }
    let model_hits = scan
        .get("model_hits")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| fixture_error("model scan", "missing model_hits array"))?;
    for hit in model_hits {
        let text = hit
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let benign = ["model.requested", "model.stream_delta", "model.completed"]
            .iter()
            .any(|kind| text.contains(kind));
        if !benign {
            return Ok(CaseReport::fail(
                CASE,
                format!("unclassified 'model' hit in module source: {hit}"),
                evidence,
            ));
        }
    }
    evidence.push(format!(
        "all {} 'model' hits are telemetry event-kind strings (model.requested / \
         model.stream_delta / model.completed) — no model registry, no model card",
        model_hits.len()
    ));
    let trace = read_trace(ctx, "selection-trace.json")?;
    let probes = trace
        .get("probes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| fixture_error("selection trace", "missing probes array"))?;
    for (i, probe) in probes.iter().enumerate() {
        let selected = probe
            .get("selected")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let satisfying = probe
            .get("satisfying_sorted")
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        evidence.push(format!(
            "probe {i}: selected={selected} (first of satisfying {{{satisfying}}}); \
             the record carries no rationale, no quality, no latency, no memory"
        ));
        for banned in ["rationale", "quality", "latency", "memory"] {
            if probe.get(banned).is_some() {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("probe {i} unexpectedly records {banned}"),
                    evidence,
                ));
            }
        }
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"tradeoff_hits": 0, "probes": probes.len()}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(ctx: &Ctx, case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "selection_strategy_reality" => Ok(run_driver_scenario(ctx, case, "selection")),
        "model_selection_absent" => Ok(run_driver_scenario(ctx, case, "modelscan")),
        "capability_needs_boolean_only" => case_capability_needs_boolean_only(ctx),
        "no_tradeoff_record" => case_no_tradeoff_record(ctx),
        _ => Err(DriverError::Fixture {
            what: "case".to_string(),
            detail: format!("unknown case '{case}'"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Task entry point
// ---------------------------------------------------------------------------

struct TaskFailure {
    where_: String,
    how: String,
    evidence: Vec<String>,
}

fn run_inner(ctx: &Ctx) -> Result<Vec<String>, TaskFailure> {
    let mut evidence = vec![
        "seam: diver's ai.harness.adapter.negotiate(adapters, needs) — selects the first adapter in sorted name order whose probed boolean capabilities satisfy every need; the vocabulary is the seven boolean CAPABILITY_KEYS, no model selection".to_string(),
    ];
    for case in CASES {
        let report = run_case(ctx, case).map_err(|e| TaskFailure {
            where_: case.to_string(),
            how: e.to_string(),
            evidence: evidence.clone(),
        })?;
        evidence.push(format!("case {case}: passed={}", report.passed));
        evidence.push(format!("case {case} metrics: {}", report.metrics));
        for line in &report.evidence {
            evidence.push(format!("case {case}: {line}"));
        }
        if !report.passed {
            return Err(TaskFailure {
                where_: case.to_string(),
                how: report.failures.join("; "),
                evidence,
            });
        }
    }
    evidence.push(
        "finding: the selector is real but solves a different problem — boolean capability coverage, not model tradeoffs; latency, memory, quality, model cards, and benchmarks are not inputs and no rationale is recorded".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: diver's ai.harness.adapter.negotiate(adapters, needs) selects the first adapter in sorted name order whose probed boolean capabilities satisfy every requested need — the negotiated vocabulary is exactly the seven boolean CAPABILITY_KEYS (streaming, cancellation, resume, permissions, artifacts, remote, tools); max_input_bytes is an optional integer the negotiation never reads. There is no model registry, no model card, no latency/memory/quality signal, no benchmark, and no rationale: the function returns the adapter table, full stop. Driver probes against the REAL module confirm it: 5 negotiate() probes all select the first-sorted satisfying adapter (zeta-slow's higher driver-side quality note never wins a tie), and a debug.getinfo-resolved source scan of adapter.lua + types.lua finds zero tradeoff-vocabulary hits — the only 'model' hits are telemetry event-kind strings (model.requested / model.stream_delta / model.completed). The design's per-tool local model selector (latency/memory/quality tradeoffs with explicit rationale) has no implementation. Diver-owned: flagged, never fixed on gauntlet authority — whether diver should gain a model selector is Matt's call.".to_string(),
        evidence,
    })
}

/// Attempt the task: selection probe first.
pub fn run(ctx: &Ctx) -> TaskOutcome {
    match run_inner(ctx) {
        Ok(evidence) => TaskOutcome::Pass {
            evidence: bound_evidence(evidence),
        },
        Err(failure) => TaskOutcome::Fail {
            where_: failure.where_,
            how: failure.how,
            evidence: bound_evidence(failure.evidence),
        },
    }
}

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"selection"`, `"modelscan"`. Unknown names make
/// the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    run_nvim_lua_driver_with_env(
        ctx,
        "task_80.lua",
        "task-80",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
