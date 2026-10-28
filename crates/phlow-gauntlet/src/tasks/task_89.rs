//! task-89: protocol version skew (rust driver + harness).
//!
//! The design asks for version negotiation under skew: client and
//! server negotiate the greatest *mutually supported* version — or
//! fail closed; a server claiming a *future* version must not unlock
//! unimplemented behavior; mid-session version changes invalidate
//! the session; garbage version strings are a typed
//! `version_negotiation_failed`, not a parse panic.
//!
//! History (the finding this task originally recorded): against
//! the hand-rolled dual-era server, the seam was
//! `McpServer::initialize`. Known versions negotiated correctly,
//! but UNKNOWN versions — garbage ("banana") and future
//! ("2999-99-99") alike — silently negotiated to the NEWEST
//! supported version with no typed error, and a mid-session
//! `initialize` was rejected (-32600) without invalidating the
//! session. The task failed honestly at `"seam"`, and the two
//! product decisions (fail closed on unknown versions? invalidate
//! on re-initialize?) were banked for the owner.
//!
//! Resolution (fleet standardization, D-113/D-200; phlow rmcp
//! migration, Stage C2): the banked decisions were answered by
//! making the question disappear. The server is modern-only
//! (2026-07-28): there is no `initialize` negotiation at all. The
//! seam is now the era guard's admission function,
//! `phlow_mcp::classify` (crates/phlow-mcp/src/guard.rs), the
//! byte-level front door in front of the rmcp service:
//!
//! - `initialize` at ANY version is refused with the typed
//!   `-32022` unsupported-version error, its `data` naming the
//!   requested version and the one supported version — garbage
//!   and future versions are refused *identically in shape* and
//!   *distinguishably by name* (the refusal echoes what was
//!   asked), which is exactly the typed fail-closed the design
//!   asked for.
//! - Modern frames (`server/discover`, `tools/*` carrying the
//!   modern `_meta`) are admitted verbatim.
//! - An `initialize` arriving after modern traffic is refused the
//!   same way: the server is stateless, so there is no session to
//!   invalidate and no version to change mid-stream — the
//!   design's invalidation requirement is met by construction.
//!
//! Four cases: two validation (legacy initialize refused typed;
//! modern frames admitted), two adversarial (garbage/future
//! refused identically; mid-session initialize still refused).
//! The task-level verdict is `pass`: the skew discipline the
//! design asks for now holds at the real seam.
//!
//! The diver client side remains diver-owned and flagged (see
//! task-86): diver's `ai.mcp.client` spoke `2024-11-05` and
//! discarded the initialize result. Under the modern-only fleet
//! that client is served by the WS3 sidecar program, not by this
//! server — out of this task's seam.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_mcp::{GuardAction, classify};
use std::fmt;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-89";
/// Human-readable name.
pub const NAME: &str = "protocol version skew";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "legacy_initialize_refused_typed",
    "modern_discover_admitted",
    "garbage_and_future_refused_identically",
    "mid_session_initialize_still_refused",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-89 driver itself (not of the code under test).
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
                write!(f, "task-89: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-89: probe {case} failed: {detail}")
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

/// The one version the migrated server speaks.
const MODERN: &str = "2026-07-28";
/// The typed refusal code for era violations.
const UNSUPPORTED: i64 = -32022;

/// Classify one JSON frame value through the real guard seam.
fn classify_value(frame: &serde_json::Value) -> Result<GuardAction, DriverError> {
    let bytes =
        serde_json::to_vec(frame).map_err(|e| fixture_error("frame", format!("serialize: {e}")))?;
    Ok(classify(&bytes))
}

/// An `initialize` frame offering `version`.
fn initialize_frame(version: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": "gauntlet", "version": "0.1.0"},
        },
    })
}

/// A modern-era frame for `method`, carrying the per-request
/// `_meta` the era requires.
fn modern_frame(method: &str) -> serde_json::Value {
    let mut params = serde_json::json!({
        "_meta": {
            "io.modelcontextprotocol/protocolVersion": MODERN,
            "io.modelcontextprotocol/clientInfo": {"name": "gauntlet", "version": "0.1.0"},
            "io.modelcontextprotocol/clientCapabilities": {},
        },
    });
    if method == "tools/call" {
        params["name"] = serde_json::json!("flow_status");
        params["arguments"] = serde_json::json!({});
    }
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": method,
        "params": params,
    })
}

/// The refusal in a `Respond` frame: `(code, requested, supported)`.
fn refusal_parts(action: &GuardAction) -> Option<(i64, serde_json::Value, serde_json::Value)> {
    let GuardAction::Respond(frame) = action else {
        return None;
    };
    let code = frame.pointer("/error/code")?.as_i64()?;
    let requested = frame.pointer("/error/data/requested")?.clone();
    let supported = frame.pointer("/error/data/supported")?.clone();
    Some((code, requested, supported))
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: `initialize` at legacy versions is refused with the typed
/// -32022 error naming the requested version and the one
/// supported version — the fail-closed answer to the banked
/// product decision.
fn case_legacy_initialize_refused_typed() -> Result<CaseReport, DriverError> {
    const CASE: &str = "legacy_initialize_refused_typed";
    let mut evidence = Vec::new();
    let mut metrics = serde_json::Map::new();
    for offered in ["2025-06-18", "2025-11-25"] {
        let action = classify_value(&initialize_frame(offered))?;
        match refusal_parts(&action) {
            Some((code, requested, supported)) => {
                evidence.push(format!(
                    "initialize {offered} refused: code {code}, requested {requested}, supported {supported}"
                ));
                if code != UNSUPPORTED
                    || requested != serde_json::json!(offered)
                    || supported != serde_json::json!([MODERN])
                {
                    return Ok(CaseReport::fail(
                        CASE,
                        format!(
                            "initialize {offered} refused with the wrong shape: code {code}, requested {requested}, supported {supported}"
                        ),
                        evidence,
                    ));
                }
                metrics.insert(offered.to_string(), serde_json::json!(code));
            }
            None => {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("initialize {offered} was not refused at the guard: {action:?}"),
                    evidence,
                ));
            }
        }
    }
    evidence.push(
        "legacy initialize fails closed with a typed refusal: the \
         banked product decision is answered — there is no silent \
         negotiation to any version"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::Value::Object(metrics),
        evidence,
    ))
}

/// V2: modern frames are admitted verbatim — discover, list, and
/// call all forward to the SDK service.
fn case_modern_discover_admitted() -> Result<CaseReport, DriverError> {
    const CASE: &str = "modern_discover_admitted";
    let mut evidence = Vec::new();
    let mut forwarded = 0;
    for method in ["server/discover", "tools/list", "tools/call"] {
        let action = classify_value(&modern_frame(method))?;
        match action {
            GuardAction::Forward(_) => {
                forwarded += 1;
                evidence.push(format!("{method} with modern _meta forwarded"));
            }
            other => {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("{method} with modern _meta was not admitted: {other:?}"),
                    evidence,
                ));
            }
        }
    }
    evidence.push(
        "the modern era needs no handshake: discover and tool \
         frames are admitted statelessly, each carrying its own \
         version in _meta"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"forwarded": forwarded}),
        evidence,
    ))
}

/// A1: garbage ("banana") and future ("2999-99-99") versions are
/// refused identically — same code, same shape — while the
/// refusal's `requested` field keeps them distinguishable by
/// name. Neither unlocks anything.
fn case_garbage_and_future_refused_identically() -> Result<CaseReport, DriverError> {
    const CASE: &str = "garbage_and_future_refused_identically";
    let mut evidence = Vec::new();
    let mut metrics = serde_json::Map::new();
    for offered in ["banana", "2999-99-99"] {
        let action = classify_value(&initialize_frame(offered))?;
        match refusal_parts(&action) {
            Some((code, requested, supported)) => {
                evidence.push(format!(
                    "offered {offered:?} refused: code {code}, requested {requested}"
                ));
                if code != UNSUPPORTED
                    || requested != serde_json::json!(offered)
                    || supported != serde_json::json!([MODERN])
                {
                    return Ok(CaseReport::fail(
                        CASE,
                        format!(
                            "{offered:?} refused with a divergent shape: code {code}, requested {requested}, supported {supported}"
                        ),
                        evidence,
                    ));
                }
                metrics.insert(offered.to_string(), serde_json::json!(code));
            }
            None => {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("{offered:?} was not refused at the guard: {action:?}"),
                    evidence,
                ));
            }
        }
    }
    evidence.push(
        "garbage and future versions are indistinguishable in \
         outcome (both fail closed, typed) and distinguishable in \
         the record (the refusal names what was asked): the \
         design's typed version_negotiation_failed exists, as \
         -32022 with requested/supported data"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::Value::Object(metrics),
        evidence,
    ))
}

/// A2: an `initialize` arriving after modern traffic is refused
/// exactly like one arriving first — and the modern traffic
/// around it is unaffected. There is no session to invalidate:
/// the design's mid-session-change hazard does not exist in a
/// stateless era.
fn case_mid_session_initialize_still_refused() -> Result<CaseReport, DriverError> {
    const CASE: &str = "mid_session_initialize_still_refused";
    let mut evidence = Vec::new();
    // Modern traffic first: the "session" is just prior frames.
    match classify_value(&modern_frame("tools/call"))? {
        GuardAction::Forward(_) => evidence.push("modern tools/call forwarded".to_string()),
        other => {
            return Ok(CaseReport::fail(
                CASE,
                format!("setup modern frame was not admitted: {other:?}"),
                evidence,
            ));
        }
    }
    // The mid-session version-change attempt.
    let action = classify_value(&initialize_frame("2025-11-25"))?;
    let code = match refusal_parts(&action) {
        Some((code, _, _)) => {
            evidence.push(format!("mid-session initialize refused with code {code}"));
            code
        }
        None => {
            return Ok(CaseReport::fail(
                CASE,
                format!("mid-session initialize was not refused: {action:?}"),
                evidence,
            ));
        }
    };
    if code != UNSUPPORTED {
        return Ok(CaseReport::fail(
            CASE,
            format!("mid-session initialize refused with {code}, not {UNSUPPORTED}"),
            evidence,
        ));
    }
    // Nothing was invalidated: modern traffic still forwards.
    match classify_value(&modern_frame("tools/call"))? {
        GuardAction::Forward(_) => {
            evidence.push("modern tools/call still forwarded after the refusal".to_string());
        }
        other => {
            return Ok(CaseReport::fail(
                CASE,
                format!("modern traffic broke after the refusal: {other:?}"),
                evidence,
            ));
        }
    }
    evidence.push(
        "the version-change attempt is refused and nothing else \
         changes: no session exists to invalidate, so the \
         design's invalidation requirement is met by construction"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"initialize_code": code, "session_invalidated": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "legacy_initialize_refused_typed" => case_legacy_initialize_refused_typed(),
        "modern_discover_admitted" => case_modern_discover_admitted(),
        "garbage_and_future_refused_identically" => case_garbage_and_future_refused_identically(),
        "mid_session_initialize_still_refused" => case_mid_session_initialize_still_refused(),
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

fn run_inner() -> Result<Vec<String>, TaskFailure> {
    let mut evidence = vec![
        "seam: phlow-mcp's era guard, phlow_mcp::classify \
         (crates/phlow-mcp/src/guard.rs) — the byte-level admission \
         in front of the rmcp service, since the fleet \
         standardization (D-113/D-200) made the server modern-only \
         and deleted the initialize negotiation this task \
         originally probed"
            .to_string(),
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
        "finding: the skew discipline holds at the modern seam — \
         every non-modern version contact fails closed with the \
         typed -32022 refusal (requested + supported named), \
         modern frames are admitted statelessly, and mid-session \
         version changes are impossible by construction. The \
         pre-migration finding (silent upgrade to newest; \
         re-initialize without invalidation) is closed by the \
         rmcp migration, Stage C2"
            .to_string(),
    );
    Ok(evidence)
}

/// Attempt the task.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_inner() {
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
