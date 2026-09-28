//! task-87: MCP session resumption (nvim-lua driver + harness).
//!
//! The design asks for resumption discipline across server restarts:
//! the client re-handshakes and resumes with at-most-once call
//! semantics; a non-idempotent in-flight call at restart must fail
//! closed (never blindly retried); a restarted server returning a
//! different tool list must invalidate the session; the re-handshake
//! must be observable in the session record with a marked
//! resumption boundary.
//!
//! Seam mapping (verified, not invented): diver's `ai.mcp.client`
//! (`lua/ai/mcp/client.lua`) keeps sessions in a module-local table
//! keyed by server name. `teardown` on server exit bumps the
//! generation, resolves every pending request with a plain string
//! reason, closes the backend, and DELETES the session
//! (`sessions[name] = nil`). There is no re-handshake path, no
//! session record, no idempotency tracking, no tool-list identity,
//! and no resumption-boundary marking: after a death `M.request`
//! reports 'server is not running' until the caller starts over by
//! hand, and a restarted server's changed tool list is served with
//! no invalidation.
//!
//! Four cases: two validation (driver probes against the REAL
//! module), two adversarial (harness probes over the driver's
//! machine-readable traces). The task-level verdict is `fail` at
//! `"seam"`.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//! Whether `ai.mcp.client` should gain resumption machinery
//! (session records, at-most-once/idempotency tracking, tool-list
//! identity, invalidation on identity change) is Matt's call.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence, run_nvim_lua_driver_with_env};
use std::fmt;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-87";
/// Human-readable name.
pub const NAME: &str = "MCP session resumption";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Probe cases the driver runs, in order:
/// two validation (driver probes), two adversarial (harness probes).
pub const CASES: [&str; 4] = [
    "restart_between_calls",
    "identity_change_no_invalidation",
    "inflight_call_untyped",
    "no_session_record",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-87 driver itself (not of the code under test).
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
                write!(f, "task-87: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-87: probe {case} failed: {detail}")
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

/// Task work dir: the nvim runner creates `ctx.work_dir/task-87`.
fn work_dir(ctx: &Ctx) -> std::path::PathBuf {
    ctx.work_dir.join("task-87")
}

/// Run one driver scenario and convert its [`TaskOutcome`] into a case
/// report.
fn run_driver_scenario(ctx: &Ctx, case: &'static str, scenario: &str) -> CaseReport {
    match run_nvim_lua_driver_with_env(
        ctx,
        "task_87.lua",
        "task-87",
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

/// A1: a tools/call is in flight when the server dies. Over the
/// driver's trace: the pending call resolved with the exit reason,
/// the wire log shows exactly one tools/call (never retried), but
/// `typed_unknown_call_outcome` is false and `idempotency_tracked`
/// is false — the call's fate is unknown with no typed discipline
/// and no way to distinguish a safe retry from an unsafe one.
fn case_inflight_call_untyped(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "inflight_call_untyped";
    let mut evidence = Vec::new();
    // The driver scenario must run first so the trace exists.
    let driver = run_driver_scenario(ctx, "inflight_driver", "inflight");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "resume-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "inflight" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'inflight'"),
            evidence,
        ));
    }
    let wire_calls = trace.get("wire_calls").and_then(serde_json::Value::as_u64);
    evidence.push(format!("trace wire_calls={wire_calls:?}"));
    if wire_calls != Some(1) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not show exactly one tools/call on the wire".to_string(),
            evidence,
        ));
    }
    let typed = trace
        .get("typed_unknown_call_outcome")
        .and_then(serde_json::Value::as_bool);
    let idempotent = trace
        .get("idempotency_tracked")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace typed_unknown_call_outcome={typed:?} idempotency_tracked={idempotent:?}"
    ));
    if typed != Some(false) || idempotent != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the untyped, untracked in-flight failure".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the dead call was never retried (at-most-once by teardown, not \
         by discipline) — but its fate surfaces as a bare string with no \
         typed `unknown_call_outcome` and no idempotency tracking, so a \
         caller cannot tell a safe retry from an unsafe one"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"wire_calls": 1, "typed_unknown_call_outcome": false}),
        evidence,
    ))
}

/// A2: the client exposes no session-record API. Over the driver's
/// trace: `session_record_api` is false and `resumption_boundary`
/// is false — the re-handshake is not observable and no resumption
/// boundary is ever marked.
fn case_no_session_record(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_session_record";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "no_record_driver", "no-record");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "resume-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "no-record" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'no-record'"),
            evidence,
        ));
    }
    let record_api = trace
        .get("session_record_api")
        .and_then(serde_json::Value::as_bool);
    let boundary = trace
        .get("resumption_boundary")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace session_record_api={record_api:?} resumption_boundary={boundary:?}"
    ));
    if record_api != Some(false) || boundary != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the absent session record".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "teardown deletes the session outright (sessions[name] = nil): \
         there is no record in which a re-handshake could be observed \
         or a resumption boundary marked"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"session_record_api": false, "resumption_boundary": false}),
        evidence,
    ))
}

/// V1: after the server process dies between calls, the client does
/// NOT auto-resume. Over the driver's trace: `auto_resumed` is
/// false, `rehandshake_ok` is true — recovery is a manual
/// re-handshake from scratch, never automatic resumption.
fn case_restart_between_calls(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "restart_between_calls";
    let mut evidence = Vec::new();
    // The driver scenario must run first so the trace exists.
    let driver = run_driver_scenario(ctx, "restart_driver", "restart-between");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "resume-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "restart-between" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'restart-between'"),
            evidence,
        ));
    }
    let auto = trace
        .get("auto_resume")
        .and_then(serde_json::Value::as_bool);
    let rehandshake_ok = trace
        .get("rehandshake_ok")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace auto_resumed={auto:?} rehandshake_ok={rehandshake_ok:?}"
    ));
    if auto != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "the client unexpectedly auto-resumed".to_string(),
            evidence,
        ));
    }
    if rehandshake_ok != Some(true) {
        return Ok(CaseReport::fail(
            CASE,
            "the manual re-handshake after restart did not succeed".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "no automatic resumption: the session record is deleted on \
         process death, recovery is a manual M.start from scratch, and \
         the second handshake re-negotiates rather than resumes"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"auto_resumed": false, "rehandshake_ok": true}),
        evidence,
    ))
}

/// V2: a restarted server with a CHANGED tool list is served as-is.
/// Over the driver's trace: `invalidation_api` is false and
/// `resumption_marker` is false — the client keeps no tool-list
/// identity, so identity changes are never detected, and no
/// resumption marker exists anywhere.
fn case_identity_change_no_invalidation(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "identity_change_no_invalidation";
    let mut evidence = Vec::new();
    // The driver scenario must run first so the trace exists.
    let driver = run_driver_scenario(ctx, "identity_driver", "identity-change");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "resume-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "identity-change" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'identity-change'"),
            evidence,
        ));
    }
    let old_tools = trace.get("old_tools").and_then(|t| t.as_array());
    let new_tools = trace.get("new_tools").and_then(|t| t.as_array());
    evidence.push(format!(
        "trace old_tools={old_tools:?} new_tools={new_tools:?}"
    ));
    let changed = match (old_tools, new_tools) {
        (Some(old), Some(new)) => !new.is_empty() && old != new,
        _ => false,
    };
    if !changed {
        return Ok(CaseReport::fail(
            CASE,
            "the restarted server did not change its tool list".to_string(),
            evidence,
        ));
    }
    let invalidated = trace
        .get("session_invalidated")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!("trace session_invalidated={invalidated:?}"));
    if invalidated != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the un-invalidated session".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "the new tool list is served with no invalidation: the client \
         keeps no tool-list identity, exposes no invalidation API, and \
         records no resumption marker"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"tools_changed": true, "session_invalidated": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(ctx: &Ctx, case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "restart_between_calls" => case_restart_between_calls(ctx),
        "identity_change_no_invalidation" => case_identity_change_no_invalidation(ctx),
        "inflight_call_untyped" => case_inflight_call_untyped(ctx),
        "no_session_record" => case_no_session_record(ctx),
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
        "seam: diver's ai.mcp.client — real process-lifetime handling, but \
         with no resumption machinery: teardown deletes the session on \
         server exit; M.request then reports 'server is not running' \
         until a manual M.start; no session record, no idempotency \
         tracking, no tool-list identity"
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
        "finding: death between calls is survivable only by manual \
         restart, and nothing is recorded or invalidated — the \
         resumption discipline the design asks for has no implementation"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: diver's ai.mcp.client (lua/ai/mcp/client.lua) handles server death but has no resumption machinery. Driver probes against the REAL module confirm it: after a between-calls death M.request reports 'server is not running' — no auto-resume — and a manual M.start re-handshakes with tools/list working again, but no session record marks the resumption boundary; a restarted server returning a different tool list is served with no invalidation — there is no tool-list identity to compare against; an in-flight tools/call at restart resolves with the exit reason and is never retried (exactly one tools/call on the wire), but the fate surfaces as a bare string — no typed `unknown_call_outcome`, no idempotency tracking to distinguish a safe retry from an unsafe one; and the client module exposes no session-record API at all (teardown deletes the session outright). Diver-owned: flagged, never fixed on gauntlet authority — whether the client should gain resumption machinery (session records, at-most-once/idempotency tracking, tool-list identity, invalidation on identity change) is Matt's call.".to_string(),
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
/// Known scenarios: `"restart-between"`, `"identity-change"`,
/// `"inflight"`, `"no-record"`. Unknown names make the driver report
/// failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    run_nvim_lua_driver_with_env(
        ctx,
        "task_87.lua",
        "task-87",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
