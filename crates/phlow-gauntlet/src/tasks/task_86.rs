//! task-86: MCP capability negotiation mismatch (nvim-lua driver + harness).
//!
//! The design asks for negotiation integrity on the MCP client
//! handshake: the client must verify that the server's *behavior*
//! matches its *advertised* capabilities, never issue calls for
//! unadvertised capabilities, surface mismatches as typed errors
//! naming the capability, and record the negotiated capability set
//! in the session record.
//!
//! Seam mapping (verified, not invented): diver's `ai.mcp.client`
//! (`lua/ai/mcp/client.lua`) owns the initialize handshake. Its
//! `run_handshake` sends `protocolVersion = '2024-11-05'` with
//! `capabilities = {}` and DISCARDS the initialize result
//! (`function(err, _result)`): the server's advertised capabilities
//! are never read, never verified, never recorded. `M.request`
//! sends any method for any ready session — no gating on negotiated
//! capabilities, no `capability_mismatch` typed error, no session
//! record of what was negotiated.
//!
//! Four cases: two validation (driver probes against the REAL
//! module), two adversarial (harness probes over the driver's
//! machine-readable traces). The task-level verdict is `fail` at
//! `"seam"`.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//! Whether `ai.mcp.client` should verify advertisement-vs-behavior,
//! gate requests on negotiated capabilities, and record the
//! negotiation is Matt's call.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence, run_nvim_lua_driver_with_env};
use std::fmt;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-86";
/// Human-readable name.
pub const NAME: &str = "MCP capability negotiation mismatch";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Probe cases the driver runs, in order:
/// two validation (driver probes), two adversarial (harness probes).
pub const CASES: [&str; 4] = [
    "matching_roundtrip",
    "negotiation_unrecorded",
    "tools_lie_untyped",
    "unadvertised_call_sent",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-86 driver itself (not of the code under test).
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
                write!(f, "task-86: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-86: probe {case} failed: {detail}")
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

/// Task work dir: the nvim runner creates `ctx.work_dir/task-86`.
fn work_dir(ctx: &Ctx) -> std::path::PathBuf {
    ctx.work_dir.join("task-86")
}

/// Run one driver scenario and convert its [`TaskOutcome`] into a case
/// report.
fn run_driver_scenario(ctx: &Ctx, case: &'static str, scenario: &str) -> CaseReport {
    match run_nvim_lua_driver_with_env(
        ctx,
        "task_86.lua",
        "task-86",
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

/// A1: the server advertises `tools` but tools/list errors -32602.
/// Over the driver's trace: the error surfaced as a plain string and
/// `typed_capability_mismatch` is false — there is no typed error
/// naming the capability, so advertisement-vs-behavior mismatches
/// are indistinguishable from ordinary call failures.
fn case_tools_lie_untyped(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "tools_lie_untyped";
    let mut evidence = Vec::new();
    // The driver scenario must run first so the trace exists.
    let driver = run_driver_scenario(ctx, "lie_driver", "lie");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "cap-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "lie" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'lie'"),
            evidence,
        ));
    }
    let error_text = trace
        .get("error_text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    evidence.push(format!("tools/list error surfaced as: {error_text}"));
    if error_text.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            "trace records no error text for the lying tools/list".to_string(),
            evidence,
        ));
    }
    let typed = trace
        .get("typed_capability_mismatch")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!("trace typed_capability_mismatch={typed:?}"));
    if typed != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the untyped error".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "an advertised-but-broken capability surfaces as a bare string: \
         no typed `capability_mismatch` names the capability, so the \
         negotiation lie is indistinguishable from an ordinary call failure"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"typed_capability_mismatch": false}),
        evidence,
    ))
}

/// A2: the server advertises only `tools`; the driver calls
/// `resources/list`. Over the driver's trace: the mock's wire log
/// proves the request was SENT — the client issues calls for
/// capabilities the server never advertised.
fn case_unadvertised_call_sent(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "unadvertised_call_sent";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "unadvertised_driver", "unadvertised");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "cap-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "unadvertised" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'unadvertised'"),
            evidence,
        ));
    }
    let sent = trace
        .get("unadvertised_call_sent")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!("trace unadvertised_call_sent={sent:?}"));
    if sent != Some(true) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not show the unadvertised call on the wire".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "M.request sends any method for any ready session: with the \
         server advertising only `tools`, a `resources/list` call went \
         out on the wire — negotiation integrity (never use anything \
         unadvertised) has no implementation"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"unadvertised_call_sent": true}),
        evidence,
    ))
}

/// V2: the client's negotiation is never recorded. Over the
/// driver's machine-readable trace: there is no capability/session
/// introspection API and the wire log shows the Lua client's fixed
/// empty capabilities — the negotiation result exists nowhere in
/// the client.
fn case_negotiation_unrecorded(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "negotiation_unrecorded";
    let mut evidence = Vec::new();
    // The driver scenario must run first so the trace exists.
    let driver = run_driver_scenario(ctx, "record_driver", "record");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "cap-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "record" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'record'"),
            evidence,
        ));
    }
    let client_caps = trace
        .get("client_advertised_caps")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let negotiated_record = trace
        .get("negotiated_record")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace client_advertised_caps={client_caps:?} negotiated_record={negotiated_record:?}"
    ));
    if client_caps != "[]" {
        return Ok(CaseReport::fail(
            CASE,
            format!("client advertised {client_caps:?}, not the fixed empty set"),
            evidence,
        ));
    }
    if negotiated_record != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the absent negotiation record".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the negotiation is never recorded: the client advertises its \
         fixed empty capabilities on the wire and keeps no record of \
         what the server advertised — the 'record' scenario confirms \
         there is no capability/session introspection API"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"client_advertised_caps": "[]", "negotiated_record": false}),
        evidence,
    ))
}

/// V1: matching capabilities round-trip cleanly against the REAL
/// client — over the driver's trace: handshake_ok, tool-list
/// contains `ping`, `tools/call` succeeds — but the client records
/// nothing: no capability/session introspection API and the wire
/// shows the client's fixed empty capabilities.
fn case_matching_roundtrip(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "matching_roundtrip";
    let mut evidence = Vec::new();
    // The driver scenario must run first so the trace exists.
    let driver = run_driver_scenario(ctx, "roundtrip_driver", "match");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "cap-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "match" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'match'"),
            evidence,
        ));
    }
    let handshake_ok = trace
        .get("handshake_ok")
        .and_then(serde_json::Value::as_bool);
    let roundtrip_ok = trace
        .get("roundtrip_ok")
        .and_then(serde_json::Value::as_bool);
    let server_advertised = trace.get("server_advertised").and_then(|t| t.as_array());
    evidence.push(format!(
        "trace handshake_ok={handshake_ok:?} roundtrip_ok={roundtrip_ok:?} \
         server_advertised={server_advertised:?}"
    ));
    if handshake_ok != Some(true)
        || roundtrip_ok != Some(true)
        || !server_advertised.is_some_and(|t| t.iter().any(|n| n.as_str() == Some("tools")))
    {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not show a clean matching-capability round trip".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the round trip works with the REAL client, but nothing is \
         recorded: the client exposes no capability/session \
         introspection API and the wire shows its fixed empty \
         capabilities (the 'record' scenario)"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"handshake_ok": true, "roundtrip_ok": true}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(ctx: &Ctx, case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "matching_roundtrip" => case_matching_roundtrip(ctx),
        "negotiation_unrecorded" => case_negotiation_unrecorded(ctx),
        "tools_lie_untyped" => case_tools_lie_untyped(ctx),
        "unadvertised_call_sent" => case_unadvertised_call_sent(ctx),
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
        "seam: diver's ai.mcp.client — the real initialize handshake, but \
         with no negotiation integrity: run_handshake sends \
         protocolVersion='2024-11-05' with capabilities={} and discards \
         the initialize result; M.request sends any method for any ready \
         session with no gating on negotiated capabilities"
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
        "finding: the handshake mechanism works (matching capabilities \
         round-trip), but negotiation integrity does not exist — the \
         client never verifies advertisement vs behavior, issues calls \
         for unadvertised capabilities, and records nothing"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: diver's ai.mcp.client (lua/ai/mcp/client.lua) is the real MCP handshake, but it has no negotiation integrity. Driver probes against the REAL module confirm it: matching capabilities round-trip (handshake + tools/list + tools/call echo all succeed); the initialize result is discarded — the client module exposes no capability/negotiation introspection, and the wire log shows the client advertised `\"capabilities\":[]` (empty Lua table encodes as a JSON array, the task-08 interop gap); a server advertising `tools` whose tools/list errors -32602 surfaces only a bare string error — no typed `capability_mismatch` names the capability; and with the server advertising only `tools`, a driver-issued `resources/list` call went out on the wire — M.request gates nothing on negotiated capabilities. Diver-owned: flagged, never fixed on gauntlet authority — whether the client should verify advertisement-vs-behavior, gate requests on negotiated capabilities, and record the negotiation is Matt's call.".to_string(),
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
/// Known scenarios: `"match"`, `"record"`, `"lie"`, `"unadvertised"`.
/// Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    run_nvim_lua_driver_with_env(
        ctx,
        "task_86.lua",
        "task-86",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
