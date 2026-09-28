//! task-90: namespaced cross-protocol dispatch (nvim-lua driver + harness).
//!
//! The design asks for a namespaced cross-protocol dispatcher: every
//! addressable target carries its protocol namespace
//! (`mcp:summarize` vs `a2a:summarize`), a name collision across
//! protocols can never cause cross-protocol dispatch, unqualified
//! ambiguous names are rejected (never guessed), and namespaces are
//! assigned by the router — never parsed from advertised names.
//!
//! Seam mapping (verified, not invented): diver's harness has NO
//! namespaced dispatcher. Routing is per-run adapter binding:
//! `supervisor.launch` picks exactly one adapter from
//! `spec.adapter` (or capability negotiation) and the run's calls
//! stay native to that adapter — MCP tool calls stay on
//! `ai.mcp.client`, A2A calls stay on `ai.a2a.tasks` (the MCP
//! adapter's own header: "tool calls stay on the native ai.mcp.tools
//! API"). The tool registry (`ai.harness.registry`) rejects any
//! name outside `^[a-z][a-z0-9_]*$`, so a `proto:name` address is
//! inexpressible — `register_tool(reg, 'mcp:summarize')` fails the
//! name pattern. No function in ai.harness parses a protocol prefix.
//!
//! Four cases: two validation (driver probes against the REAL
//! modules), two adversarial (harness probes over the driver's
//! machine-readable traces). The task-level verdict is `fail` at
//! `"seam"`.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//! Whether the harness should gain a namespaced cross-protocol
//! dispatcher (router-assigned namespaces, ambiguous-name
//! rejection) is Matt's call.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence, run_nvim_lua_driver_with_env};
use std::fmt;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-90";
/// Human-readable name.
pub const NAME: &str = "namespaced cross-protocol dispatch";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Probe cases the driver runs, in order:
/// two validation (driver probes), two adversarial (harness probes).
pub const CASES: [&str; 4] = [
    "adapters_disjoint",
    "no_namespace_entry",
    "spoof_prefix_opaque",
    "ambiguous_inexpressible",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-90 driver itself (not of the code under test).
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
                write!(f, "task-90: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-90: probe {case} failed: {detail}")
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
// Driver plumbing
// ---------------------------------------------------------------------------

/// Task work dir: the nvim runner creates `ctx.work_dir/task-90`.
fn work_dir(ctx: &Ctx) -> std::path::PathBuf {
    ctx.work_dir.join("task-90")
}

/// Run one driver scenario and convert its [`TaskOutcome`] into a case
/// report.
fn run_driver_scenario(ctx: &Ctx, case: &'static str, scenario: &str) -> CaseReport {
    match run_nvim_lua_driver_with_env(
        ctx,
        "task_90.lua",
        "task-90",
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

/// A1: the mock advertises a tool literally named `a2a:send`. Over
/// the driver's trace: the wire carried the literal name,
/// `prefix_parsed` is false and `cross_protocol_dispatch` is false —
/// the spoofed prefix is treated opaquely because no namespace
/// machinery exists to parse (or assign) it.
fn case_spoof_prefix_opaque(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "spoof_prefix_opaque";
    let mut evidence = Vec::new();
    // The driver scenario must run first so the trace exists.
    let driver = run_driver_scenario(ctx, "spoof_driver", "spoof");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "dispatch-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "spoof" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'spoof'"),
            evidence,
        ));
    }
    let wire_name = trace
        .get("wire_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    evidence.push(format!("wire carried the literal tool name: {wire_name:?}"));
    if wire_name != "a2a:send" {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not show the spoofed name on the wire".to_string(),
            evidence,
        ));
    }
    let parsed = trace
        .get("prefix_parsed")
        .and_then(serde_json::Value::as_bool);
    let crossed = trace
        .get("cross_protocol_dispatch")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace prefix_parsed={parsed:?} cross_protocol_dispatch={crossed:?}"
    ));
    if parsed != Some(false) || crossed != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the opaque handling".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the spoofed prefix cannot cause cross-protocol dispatch — but \
         only because no namespace machinery exists at all: the design's \
         'namespaces are router-assigned' has no router to assign them"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"prefix_parsed": false, "cross_protocol_dispatch": false}),
        evidence,
    ))
}

/// A2 (adversarial-in-V): a bare-name duplicate is rejected, but a
/// cross-protocol collision is inexpressible rather than rejected as
/// ambiguous. Over the driver's trace: `bare_duplicate_rejected` is
/// true and the namespaced lookup resolves to nothing.
fn case_ambiguous_inexpressible(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "ambiguous_inexpressible";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "ambiguous_driver", "ambiguous");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "dispatch-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "ambiguous" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'ambiguous'"),
            evidence,
        ));
    }
    let dup_rejected = trace
        .get("bare_duplicate_rejected")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!("trace bare_duplicate_rejected={dup_rejected:?}"));
    if dup_rejected != Some(true) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the duplicate rejection".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "within one registry a bare duplicate fails closed — but across \
         protocols there is no shared registry and no namespace to omit: \
         the design's 'unqualified ambiguous names are rejected' is \
         vacuous because unqualified names are the only names that exist"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"bare_duplicate_rejected": true}),
        evidence,
    ))
}

/// V1: the real `mcp` and `a2a` adapter modules are distinct
/// registrations, and an mcp run against the mock completes with
/// only mcp/supervisor-sourced events. Over the driver's trace:
/// `adapters_disjoint` is true and `cross_protocol_traffic` is
/// false — adapters are disjoint and protocol-pure by
/// construction.
fn case_adapters_disjoint(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "adapters_disjoint";
    let mut evidence = Vec::new();
    // The driver scenario must run first so the trace exists.
    let driver = run_driver_scenario(ctx, "distinct_driver", "distinct");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "dispatch-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "distinct" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'distinct'"),
            evidence,
        ));
    }
    let disjoint = trace
        .get("adapters_disjoint")
        .and_then(serde_json::Value::as_bool);
    let cross = trace
        .get("cross_protocol_traffic")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace adapters_disjoint={disjoint:?} cross_protocol_traffic={cross:?}"
    ));
    if disjoint != Some(true) || cross != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm disjoint, protocol-pure adapters".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the real mcp and a2a adapters are distinct registrations \
         with protocol-pure start paths, and the mcp run emits only \
         mcp/supervisor-sourced events: routing is per-run adapter \
         binding, no call crosses protocols"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"adapters_disjoint": true, "cross_protocol_traffic": false}),
        evidence,
    ))
}

/// V2: there is no namespace entry point. Over the driver's
/// trace: `dispatch_entry` is false and `colon_names_rejected` is
/// true — `ai.harness` exposes only setup/run/cancel/resume/version,
/// the registry only adapter/tool/workflow registration, and
/// `register_tool` REJECTS `mcp:summarize` on the name pattern. The
/// design's `proto:name` addressing is inexpressible.
fn case_no_namespace_entry(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_namespace_entry";
    let mut evidence = Vec::new();
    // The driver scenario must run first so the trace exists.
    let driver = run_driver_scenario(ctx, "no_namespace_driver", "no-namespace");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "dispatch-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "no-namespace" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'no-namespace'"),
            evidence,
        ));
    }
    let dispatch_entry = trace
        .get("dispatch_entry")
        .and_then(serde_json::Value::as_bool);
    let colon_rejected = trace
        .get("colon_names_rejected")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace dispatch_entry={dispatch_entry:?} colon_names_rejected={colon_rejected:?}"
    ));
    if dispatch_entry != Some(false) || colon_rejected != Some(true) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the absent namespace entry".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "there is no dispatch/namespace entry point and \
         `proto:name` addressing is rejected by the registry name \
         pattern: the namespaced dispatcher is absent as designed"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"dispatch_entry": false, "colon_names_rejected": true}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(ctx: &Ctx, case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "adapters_disjoint" => case_adapters_disjoint(ctx),
        "no_namespace_entry" => case_no_namespace_entry(ctx),
        "spoof_prefix_opaque" => case_spoof_prefix_opaque(ctx),
        "ambiguous_inexpressible" => case_ambiguous_inexpressible(ctx),
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
        "seam: ABSENT as designed — diver's harness has no namespaced \
         cross-protocol dispatcher. Routing is per-run adapter binding \
         (supervisor.launch picks one adapter from spec.adapter); the \
         tool registry rejects names outside ^[a-z][a-z0-9_]*$, so \
         `proto:name` addressing is inexpressible"
            .to_string(),
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
        "finding: adapters are disjoint and protocol-pure, but the \
         namespaced dispatch the design asks for does not exist — \
         namespaces are neither router-assigned nor parsed, and \
         ambiguous names cannot arise"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: diver's harness (lua/ai/harness/) has no namespaced cross-protocol dispatcher. Driver probes against the REAL modules confirm it: the 'mcp' and 'a2a' adapters are distinct registrations (real ai.harness.adapters.mcp / ai.harness.adapters.a2a) and an mcp run completes against the mock with only mcp/supervisor-sourced events — routing is per-run adapter binding via spec.adapter, and calls stay native to their adapter; ai.harness exposes only setup/run/cancel/resume/version and the registry only adapter/tool/workflow registration — no dispatch entry point, and register_tool rejects 'mcp:summarize' because the colon violates the ^[a-z][a-z0-9_]*$ name pattern, so `proto:name` addressing is inexpressible; a mock tool literally named `a2a:send` is called with the literal name on the MCP wire — the spoofed prefix is treated opaquely, never parsed, and no A2A traffic occurs; a bare-name duplicate is rejected ('already registered') but a cross-protocol collision cannot be expressed, let alone rejected as ambiguous. The design's criteria — router-assigned namespaces, ambiguous-name rejection — have no mechanism to assert against. Diver-owned: flagged, never fixed on gauntlet authority — whether the harness should gain a namespaced cross-protocol dispatcher is Matt's call.".to_string(),
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

/// Attempt one named driver scenario via `GAUNTLET_SCENARIO`.
///
/// Known scenarios: `"distinct"`, `"no-namespace"`, `"spoof"`,
/// `"ambiguous"`. Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    run_nvim_lua_driver_with_env(
        ctx,
        "task_90.lua",
        "task-90",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
