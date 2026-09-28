//! task-77: quantization behavior change (rust).
//!
//! The design asks for the model loading / quantization selection seam:
//! the same model at FP16 vs INT8 vs INT4 is not the same model
//! behaviorally — structured outputs (tool-call JSON) degrade first,
//! and silently. The harness must MEASURE per-quantization quality on
//! fixtures (never assume parity), validate every tool call against
//! the JSON schema (rejecting malformed calls with the violation
//! named, never executing them), and record the selected quantization
//! level in the run record.
//!
//! Seam recon (verified, not invented):
//! - phlow has NO model weight quantization selection. There is no
//!   model loading path at all: models are served by Ollama over HTTP
//!   (`phlow-llm`: `POST {base}/v1/chat/completions`, `GET
//!   /api/tags`). No `Cargo.toml` in the workspace depends on
//!   `bitsandbytes`, GPTQ, AWQ, or any quantization toolkit.
//! - The only quantization in phlow is `phlow_inference::kv_policy`:
//!   a KV-CACHE quantization policy (FP4/FP8/...) derived from a
//!   paper — a planning/analysis artifact (`validate` checks the
//!   paper's rules, `recommend` returns the paper's per-component
//!   policy, `bytes_per_token` estimates footprint). It is never
//!   consulted at model-load time, because there is no model-load
//!   time.
//! - The tool-call VALIDATION path is real and works: the runtime's
//!   `chat()` rejects bad message shapes, `normalize_tool_calls`
//!   rejects malformed calls / bad ids, `parse_tool_arguments`
//!   rejects unparseable argument strings, `call_tool` requires an
//!   object and runs `phlow_mcp::validate_arguments` against the
//!   tool's schema — violations render as tool errors, never as
//!   executed calls.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented on gauntlet
//! authority): whether phlow should measure or verify the precision
//! of Ollama-served models (per-quantization fixture scores,
//! recording the served precision in the run record). That is a new
//! product feature, not a bug fix.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_inference::kv_policy::{KvComponent, Precision, recommend, validate};
use serde_json::Value;
use std::fmt;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-77";
/// Human-readable name.
pub const NAME: &str = "quantization behavior change";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "fp16_baseline_tool_calls_valid",
    "degradation_measured_per_level",
    "almost_valid_tool_calls_rejected",
    "quantization_level_not_recorded",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-77 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-77: cannot build fixture {what}: {detail}")
            }
        }
    }
}

impl std::error::Error for DriverError {}

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
// Fixtures: simulated quantization-degraded tool-call outputs
// ---------------------------------------------------------------------------

/// Fixture tool schema, mirroring the tool-parameters shape the real
/// `validate_arguments` checks (object with a required string
/// `query` and an optional integer `limit`).
fn tool_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "query": {"type": "string"},
            "limit": {"type": "integer"},
        },
        "required": ["query"],
    })
}

/// One simulated model-emitted tool call, in Ollama message shape.
fn tool_call(id: Option<&str>, name: &str, arguments: Value) -> Value {
    let function = serde_json::json!({
        "name": name,
        "arguments": arguments,
    });
    let mut call = serde_json::Map::new();
    if let Some(id) = id {
        call.insert("id".to_string(), Value::String(id.to_string()));
    }
    call.insert("function".to_string(), function);
    Value::Object(call)
}

/// FP16-level fixtures: clean tool-call JSON, all valid.
fn fp16_fixtures() -> Vec<Value> {
    vec![
        tool_call(
            Some("c1"),
            "web_search",
            serde_json::json!({"query": "rust tokenizer", "limit": 5}),
        ),
        // Ollama sends arguments as a JSON-encoded string.
        tool_call(
            Some("c2"),
            "web_search",
            Value::String("{\"query\": \"rust tokenizer\"}".to_string()),
        ),
        // Missing id: normalization assigns one (mirrors Python setdefault).
        tool_call(
            None,
            "web_search",
            serde_json::json!({"query": "rust tokenizer"}),
        ),
        tool_call(
            Some("c4"),
            "web_search",
            serde_json::json!({"query": "x", "limit": 1}),
        ),
    ]
}

/// INT8-level fixtures: minor corruption — one renamed arg.
fn int8_fixtures() -> Vec<Value> {
    vec![
        tool_call(
            Some("c1"),
            "web_search",
            serde_json::json!({"query": "rust tokenizer"}),
        ),
        // Renamed arg: valid JSON, schema-invalid (missing `query`,
        // unknown `qurey`).
        tool_call(
            Some("c2"),
            "web_search",
            serde_json::json!({"qurey": "rust tokenizer"}),
        ),
        tool_call(
            Some("c3"),
            "web_search",
            serde_json::json!({"query": "rust tokenizer", "limit": 3}),
        ),
        tool_call(
            Some("c4"),
            "web_search",
            Value::String("{\"query\": \"ok\"}".to_string()),
        ),
    ]
}

/// INT4-level fixtures: severe corruption — dropped quotes/braces,
/// mistyped values.
fn int4_fixtures() -> Vec<Value> {
    vec![
        // Dropped quote: unparseable arguments string.
        tool_call(
            Some("c1"),
            "web_search",
            Value::String("{\"query\": \"rust tokenizer}".to_string()),
        ),
        // Dropped brace: unparseable arguments string.
        tool_call(
            Some("c2"),
            "web_search",
            Value::String("{\"query\": \"rust\"".to_string()),
        ),
        tool_call(
            Some("c3"),
            "web_search",
            serde_json::json!({"query": "survivor"}),
        ),
        // Wrong type: valid JSON, schema-invalid.
        tool_call(Some("c4"), "web_search", serde_json::json!({"query": 42})),
    ]
}

/// Run one raw tool call through the runtime's real validation
/// pipeline: shape (`parse_chat_message`), ids (`normalize_tool_calls`),
/// argument parsing (mirrors private `parse_tool_arguments`), object
/// check + schema (`validate_arguments`, mirrors `call_tool`).
/// Returns `Ok(())` when the call would be dispatched, else the stage
/// and the named violation.
fn validate_pipeline(call: &Value, schema: &Value) -> Result<(), String> {
    let response = serde_json::json!({
        "choices": [{"message": {"content": "", "tool_calls": [call]}}],
    });
    phlow_llm::parse_chat_message(&response).map_err(|e| format!("shape: {e}"))?;
    let normalized = phlow_runtime::normalize_tool_calls(std::slice::from_ref(call), "task-77")
        .map_err(|e| format!("normalize: {e}"))?;
    let function = normalized[0]
        .get("function")
        .and_then(|f| f.as_object())
        .ok_or_else(|| "normalize: function not an object".to_string())?;
    // Mirrors parse_tool_arguments: strings are JSON-decoded, other
    // values pass through unchanged.
    let parsed = match function.get("arguments") {
        None => Value::Object(serde_json::Map::new()),
        Some(Value::String(text)) => serde_json::from_str(text)
            .map_err(|e| format!("arguments: Invalid tool arguments: {e}"))?,
        Some(other) => other.clone(),
    };
    // Mirrors call_tool's object check.
    if !parsed.is_object() {
        return Err("arguments: Tool arguments must be an object".to_string());
    }
    phlow_mcp::validate_arguments(&parsed, schema).map_err(|e| format!("schema: {e}"))?;
    Ok(())
}

/// Schema-valid JSON rate for a fixture set: fixtures passing the
/// whole pipeline over total fixtures.
fn valid_rate(fixtures: &[Value], schema: &Value, evidence: &mut Vec<String>) -> f64 {
    let mut valid = 0usize;
    for (i, call) in fixtures.iter().enumerate() {
        match validate_pipeline(call, schema) {
            Ok(()) => {
                valid += 1;
                evidence.push(format!("fixture {i}: valid"));
            }
            Err(stage) => evidence.push(format!("fixture {i}: rejected at {stage}")),
        }
    }
    valid as f64 / fixtures.len() as f64
}

/// V1: the FP16 default — clean tool-call JSON is fully valid through
/// the real pipeline (shape, ids, argument parsing, schema). This is
/// the mechanism baseline the degraded levels are measured against.
fn case_fp16_baseline_tool_calls_valid() -> Result<CaseReport, DriverError> {
    const CASE: &str = "fp16_baseline_tool_calls_valid";
    let mut evidence = Vec::new();
    let schema = tool_schema();
    let rate = valid_rate(&fp16_fixtures(), &schema, &mut evidence);
    evidence.push(format!(
        "FP16 fixtures: schema-valid rate {rate} (4/4 through parse_chat_message + \
         normalize_tool_calls + parse_tool_arguments + validate_arguments)"
    ));
    if rate != 1.0 {
        return Ok(CaseReport::fail(
            CASE,
            format!("FP16 baseline not fully valid: rate {rate}"),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"level": "FP16", "valid_rate": rate}),
        evidence,
    ))
}

/// V2: the design's core demand — per-quantization quality MEASURED on
/// fixtures, not assumed. The driver measures the schema-valid rate
/// per simulated level and requires the degradation ordering
/// FP16 > INT8 > INT4.
fn case_degradation_measured_per_level() -> Result<CaseReport, DriverError> {
    const CASE: &str = "degradation_measured_per_level";
    let mut evidence = Vec::new();
    let schema = tool_schema();
    let fp16 = valid_rate(&fp16_fixtures(), &schema, &mut evidence);
    let int8 = valid_rate(&int8_fixtures(), &schema, &mut evidence);
    let int4 = valid_rate(&int4_fixtures(), &schema, &mut evidence);
    evidence.push(format!(
        "measured schema-valid rates: FP16={fp16}, INT8={int8}, INT4={int4}"
    ));
    if !(fp16 > int8 && int8 > int4) {
        return Ok(CaseReport::fail(
            CASE,
            format!("no measured degradation ordering: FP16={fp16}, INT8={int8}, INT4={int4}"),
            evidence,
        ));
    }
    evidence.push(
        "degradation is measured per level on fixtures (FP16 4/4, INT8 3/4, INT4 \
         1/4) — the harness never assumes parity; phlow itself performs no such \
         measurement because there is no quantization selection to measure for"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"fp16": fp16, "int8": int8, "int4": int4}),
        evidence,
    ))
}

/// A1: INT4-style almost-valid tool calls — dropped quotes, renamed
/// args, non-object arguments, duplicate ids, non-object function.
/// Every one is rejected with the schema/shape violation NAMED; none
/// is executed.
fn case_almost_valid_tool_calls_rejected() -> Result<CaseReport, DriverError> {
    const CASE: &str = "almost_valid_tool_calls_rejected";
    let mut evidence = Vec::new();
    let schema = tool_schema();
    let adversarial: [(&str, Value, &str); 5] = [
        (
            "dropped quote",
            tool_call(
                Some("a1"),
                "web_search",
                Value::String("{\"query\": \"rust tokenizer}".to_string()),
            ),
            "Invalid tool arguments",
        ),
        (
            "renamed arg",
            tool_call(
                Some("a2"),
                "web_search",
                serde_json::json!({"qurey": "rust tokenizer"}),
            ),
            "unknown arguments: ['qurey']",
        ),
        (
            "non-object arguments",
            tool_call(Some("a3"), "web_search", serde_json::json!(42)),
            "Tool arguments must be an object",
        ),
        (
            "non-object function",
            serde_json::json!({"id": "a4", "function": "nope"}),
            "Malformed function call",
        ),
        (
            "missing required query",
            tool_call(Some("a5"), "web_search", serde_json::json!({"limit": 3})),
            "Missing arguments: ['query']",
        ),
    ];
    for (label, call, want) in adversarial {
        match validate_pipeline(&call, &schema) {
            Ok(()) => {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("{label}: malformed call PASSED validation — executed malformed"),
                    evidence,
                ));
            }
            Err(stage) => {
                evidence.push(format!("{label}: rejected at {stage}"));
                if !stage.contains(want) {
                    return Ok(CaseReport::fail(
                        CASE,
                        format!("{label}: violation not named (want {want:?}, got {stage:?})"),
                        evidence,
                    ));
                }
            }
        }
    }
    // Duplicate ids need two calls in one normalize batch.
    let dup_a = tool_call(Some("dup"), "web_search", serde_json::json!({"query": "x"}));
    let dup_b = tool_call(Some("dup"), "web_search", serde_json::json!({"query": "y"}));
    match phlow_runtime::normalize_tool_calls(&[dup_a, dup_b], "task-77") {
        Ok(_) => {
            return Ok(CaseReport::fail(
                CASE,
                "duplicate ids: PASSED normalization".to_string(),
                evidence,
            ));
        }
        Err(e) => {
            evidence.push(format!("duplicate ids: rejected at normalize: {e}"));
            if !e.contains("Duplicate tool call ids") {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("duplicate ids: violation not named: {e}"),
                    evidence,
                ));
            }
        }
    }
    evidence.push(
        "every malformed tool call is rejected with the violation named \
         (shape / normalize / arguments / schema stage) — the validation path \
         exists and holds; nothing malformed reaches execution"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"adversarial_rejected": 6}),
        evidence,
    ))
}

/// A2: the design wants the selected quantization level recorded in
/// the run record. There is no selection: the only quantization in
/// phlow is the KV-cache policy, a paper-derived planning artifact
/// never consulted at load time.
fn case_quantization_level_not_recorded() -> Result<CaseReport, DriverError> {
    const CASE: &str = "quantization_level_not_recorded";
    let mut evidence = Vec::new();
    // The only quantization in phlow: KV-cache policy from a paper.
    let policy = recommend(KvComponent::GlobalMain);
    let violations = validate(&policy);
    evidence.push(format!(
        "phlow_inference::kv_policy::recommend(GlobalMain) = {:?} (precision {:?}); \
         validate() violations = {}",
        policy.component,
        policy.precision,
        violations.len()
    ));
    if policy.precision != Precision::Fp4 {
        return Ok(CaseReport::fail(
            CASE,
            "paper recommendation changed: fixture broken".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the policy is KV-cache quantization (bytes/token footprint), not model \
         weight quantization: no bitsandbytes/GPTQ/AWQ-style selection exists, \
         models load inside Ollama, and nothing in phlow consults this policy \
         at load time — there is no load time"
            .to_string(),
    );
    evidence.push(
        "the run report records model_calls (a counter) and role state; no \
         quantization/precision field exists anywhere in the report schema — \
         the design's \"selected quantization level is recorded\" has no \
         selection to record"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"quantization_selection_exists": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "fp16_baseline_tool_calls_valid" => case_fp16_baseline_tool_calls_valid(),
        "degradation_measured_per_level" => case_degradation_measured_per_level(),
        "almost_valid_tool_calls_rejected" => case_almost_valid_tool_calls_rejected(),
        "quantization_level_not_recorded" => case_quantization_level_not_recorded(),
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

fn run_inner(_ctx: &Ctx) -> Result<Vec<String>, TaskFailure> {
    let mut evidence = vec![
        "recon: no model weight quantization selection exists in any phlow crate — models are served by Ollama over HTTP (phlow-llm: POST {base}/v1/chat/completions); no bitsandbytes/GPTQ/AWQ dependency in any Cargo.toml".to_string(),
        "recon: the only quantization is phlow_inference::kv_policy — KV-cache precision policy (FP4/FP8) derived from a paper; validate() checks paper rules, recommend() returns the paper's policy, bytes_per_token() estimates footprint; never consulted at model-load time".to_string(),
        "recon: the tool-call validation path is real — chat() rejects bad message shapes, normalize_tool_calls rejects malformed calls/bad ids, parse_tool_arguments rejects unparseable argument strings, call_tool requires an object and runs phlow_mcp::validate_arguments against the tool schema".to_string(),
    ];
    for case in CASES {
        let report = run_case(case).map_err(|e| TaskFailure {
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
        "finding: per-quantization quality is never measured by phlow (there is no quantization selection to measure for — the KV-cache policy is not a load-time control); malformed tool calls ARE rejected with named violations (the validation path works); no quantization level is recorded because none is selected".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: phlow has no model loading / quantization selection seam — models are served by Ollama over HTTP and Ollama owns precision; the only quantization in the workspace is phlow_inference::kv_policy, a KV-cache precision policy derived from a paper (validate/recommend/bytes_per_token), never consulted at model-load time because there is no model-load time. Per-quantization quality numbers are therefore never measured (nothing to measure for), and no quantization level is recorded in the run record (nothing is selected). The one testable mechanism — tool-call validation — works: the driver measured FP16 4/4, INT8 3/4, INT4 1/4 schema-valid rates on fixtures and verified six INT4-style corruptions (dropped quote, renamed arg, non-object arguments, non-object function, missing required arg, duplicate ids) are each rejected with the violation named, never executed. Whether phlow should measure or verify Ollama-served model precision (per-quantization fixture scores, recording served precision in the run record) is a product decision for Matt, not a gauntlet-authorized change.".to_string(),
        evidence,
    })
}

/// Attempt the task.
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
