//! task-89: protocol version skew (rust driver + harness).
//!
//! The design asks for version negotiation under skew: client and
//! server negotiate the greatest *mutually supported* version — or
//! fail closed; a server claiming a *future* version must not unlock
//! unimplemented behavior; mid-session version changes invalidate
//! the session; garbage version strings are a typed
//! `version_negotiation_failed`, not a parse panic.
//!
//! Seam mapping (verified, not invented): the rust seam is
//! `phlow-mcp`'s `McpServer::initialize`
//! (`crates/phlow-mcp/src/server.rs`), the real MCP server-side
//! negotiation. It negotiates KNOWN versions correctly (client
//! "2025-06-18" → "2025-06-18"), but UNKNOWN versions — garbage
//! ("banana") and future ("2999-99-99") alike — silently negotiate
//! to the NEWEST supported version (`PROTOCOL_VERSION =
//! "2025-11-25"`) with no typed error: the server cannot distinguish
//! a future version from garbage, and a client implementing only an
//! older version is told the server speaks the newest. A second
//! `initialize` mid-session is rejected ("Already initialized",
//! -32600) but the session is NOT invalidated — it continues on the
//! old version.
//!
//! The diver client side (diver-owned, flagged): `ai.mcp.client`
//! sends `protocolVersion = '2024-11-05'` — older than every version
//! phlow-mcp supports — and DISCARDS the initialize result, so the
//! negotiated version never reaches any feature gate. Skew is
//! silently accepted on both ends.
//!
//! Four cases: two validation (known-version negotiation, which the
//! server gets right), two adversarial (garbage/future versions,
//! mid-session change). The task-level verdict is `fail` at
//! `"seam"`.
//!
//! Product-decision bank (phlow-owned, not implemented on gauntlet
//! authority): whether unknown versions should fail closed (-32602)
//! instead of silently upgrading to newest, and whether a
//! mid-session re-initialize should invalidate the session.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_mcp::server::{FakeRuntime, McpServer};
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
    "matching_versions_negotiate",
    "skew_negotiates_older",
    "garbage_and_future_silently_upgrade",
    "mid_session_change_not_invalidated",
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

/// The newest version phlow-mcp supports (mirrors
/// `phlow_mcp::protocol::PROTOCOL_VERSION`; read from the reply, not
/// assumed, wherever the assertion needs it).
const NEWEST: &str = "2025-11-25";

/// Send one `initialize` with `version` and return the parsed reply.
fn initialize_reply(
    server: &mut McpServer<FakeRuntime>,
    version: &str,
) -> Result<serde_json::Value, DriverError> {
    let frame = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": "gauntlet", "version": "0.1.0"},
        },
    });
    let bytes = serde_json::to_vec(&frame)
        .map_err(|e| fixture_error("initialize frame", format!("serialize: {e}")))?;
    let reply = server
        .reply(&bytes)
        .ok_or_else(|| fixture_error("initialize reply", "server sent no reply"))?;
    serde_json::from_slice(&reply)
        .map_err(|e| fixture_error("initialize reply", format!("unparseable: {e}")))
}

/// The negotiated `protocolVersion` from an initialize result, or the
/// JSON-RPC error code when the reply is an error.
fn negotiated_or_code(reply: &serde_json::Value) -> Result<String, i64> {
    if let Some(code) = reply
        .pointer("/error/code")
        .and_then(serde_json::Value::as_i64)
    {
        return Err(code);
    }
    reply
        .pointer("/result/protocolVersion")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(-1)
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: matching versions negotiate to the offered version.
fn case_matching_versions_negotiate() -> Result<CaseReport, DriverError> {
    const CASE: &str = "matching_versions_negotiate";
    let mut server = McpServer::new(FakeRuntime::ok());
    let reply = initialize_reply(&mut server, NEWEST)?;
    let mut evidence = Vec::new();
    match negotiated_or_code(&reply) {
        Ok(negotiated) => {
            evidence.push(format!("offered {NEWEST}, negotiated {negotiated}"));
            if negotiated != NEWEST {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("matching version negotiated to {negotiated}, not {NEWEST}"),
                    evidence,
                ));
            }
        }
        Err(code) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("matching version rejected with code {code}"),
                evidence,
            ));
        }
    }
    evidence
        .push("matching versions proceed: the negotiation works for known versions".to_string());
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"offered": NEWEST, "negotiated": NEWEST}),
        evidence,
    ))
}

/// V2: skew (client N-1) negotiates the greatest mutually supported
/// version — the server gets the known-version case right.
fn case_skew_negotiates_older() -> Result<CaseReport, DriverError> {
    const CASE: &str = "skew_negotiates_older";
    let mut server = McpServer::new(FakeRuntime::ok());
    let reply = initialize_reply(&mut server, "2025-06-18")?;
    let mut evidence = Vec::new();
    match negotiated_or_code(&reply) {
        Ok(negotiated) => {
            evidence.push(format!("offered 2025-06-18, negotiated {negotiated}"));
            if negotiated != "2025-06-18" {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("skew negotiated to {negotiated}, not 2025-06-18"),
                    evidence,
                ));
            }
        }
        Err(code) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("known older version rejected with code {code}"),
                evidence,
            ));
        }
    }
    evidence.push(
        "client N, server N-1 negotiates N-1 and records it in the \
         initialize result: the supported-version path is correct"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"offered": "2025-06-18", "negotiated": "2025-06-18"}),
        evidence,
    ))
}

/// A1: garbage ("banana") and future ("2999-99-99") versions both
/// silently negotiate to the NEWEST supported version — no typed
/// error, no fail-closed. The server cannot distinguish a future
/// version from garbage, and a client implementing only an older
/// version is told the server speaks the newest.
fn case_garbage_and_future_silently_upgrade() -> Result<CaseReport, DriverError> {
    const CASE: &str = "garbage_and_future_silently_upgrade";
    let mut evidence = Vec::new();
    let mut metrics = serde_json::Map::new();
    for offered in ["banana", "2999-99-99"] {
        let mut server = McpServer::new(FakeRuntime::ok());
        let reply = initialize_reply(&mut server, offered)?;
        match negotiated_or_code(&reply) {
            Ok(negotiated) => {
                evidence.push(format!(
                    "offered {offered:?}, negotiated {negotiated} (no error)"
                ));
                if negotiated != NEWEST {
                    return Ok(CaseReport::fail(
                        CASE,
                        format!("{offered:?} negotiated to {negotiated}, not {NEWEST}"),
                        evidence,
                    ));
                }
                metrics.insert(offered.to_string(), serde_json::json!(negotiated));
            }
            Err(code) => {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("{offered:?} was rejected with code {code} — typed rejection exists?"),
                    evidence,
                ));
            }
        }
    }
    evidence.push(
        "garbage and future versions are indistinguishable to the \
         server: both silently 'negotiate' the newest version. The \
         design's typed `version_negotiation_failed` does not exist, \
         and a future version unlocks the newest behavior for a client \
         that may not implement it"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::Value::Object(metrics),
        evidence,
    ))
}

/// A2: a mid-session version change does not invalidate the session.
/// After initialize("2025-06-18") + notifications/initialized, a
/// second initialize("2025-11-25") is rejected ("Already
/// initialized", -32600) but the session continues on the old
/// version — ping still succeeds.
fn case_mid_session_change_not_invalidated() -> Result<CaseReport, DriverError> {
    const CASE: &str = "mid_session_change_not_invalidated";
    let mut evidence = Vec::new();
    let mut server = McpServer::new(FakeRuntime::ok());
    let reply = initialize_reply(&mut server, "2025-06-18")?;
    if negotiated_or_code(&reply) != Ok("2025-06-18".to_string()) {
        return Ok(CaseReport::fail(
            CASE,
            format!("setup initialize failed: {reply}"),
            evidence,
        ));
    }
    let notified = server.reply(br#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#);
    if notified.is_some() {
        return Ok(CaseReport::fail(
            CASE,
            "notifications/initialized unexpectedly produced a reply".to_string(),
            evidence,
        ));
    }
    if !server.is_ready() {
        return Ok(CaseReport::fail(
            CASE,
            "server not ready after the handshake".to_string(),
            evidence,
        ));
    }
    evidence.push("session established at 2025-06-18, ready".to_string());
    // Mid-session version change attempt.
    let reply2 = initialize_reply(&mut server, "2025-11-25")?;
    match negotiated_or_code(&reply2) {
        Err(code) => {
            evidence.push(format!("second initialize rejected with code {code}"));
            if code != -32600 {
                return Ok(CaseReport::fail(
                    CASE,
                    format!("second initialize rejected with {code}, not -32600"),
                    evidence,
                ));
            }
        }
        Ok(negotiated) => {
            return Ok(CaseReport::fail(
                CASE,
                format!("second initialize unexpectedly negotiated {negotiated}"),
                evidence,
            ));
        }
    }
    // The session is NOT invalidated: it continues on the old version.
    if !server.is_ready() {
        return Ok(CaseReport::fail(
            CASE,
            "session was invalidated by the second initialize".to_string(),
            evidence,
        ));
    }
    let ping = server
        .reply(br#"{"jsonrpc": "2.0", "id": 9, "method": "ping"}"#)
        .ok_or_else(|| fixture_error("ping", "no reply after the version-change attempt"))?;
    let ping_text =
        String::from_utf8(ping).map_err(|e| fixture_error("ping", format!("not utf-8: {e}")))?;
    if !ping_text.contains("\"result\"") {
        return Ok(CaseReport::fail(
            CASE,
            format!("ping failed after the version-change attempt: {ping_text}"),
            evidence,
        ));
    }
    evidence.push(
        "the version-change attempt is rejected but the session \
         survives on the old version: the design's invalidation does \
         not happen"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"second_initialize_code": -32600, "session_invalidated": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "matching_versions_negotiate" => case_matching_versions_negotiate(),
        "skew_negotiates_older" => case_skew_negotiates_older(),
        "garbage_and_future_silently_upgrade" => case_garbage_and_future_silently_upgrade(),
        "mid_session_change_not_invalidated" => case_mid_session_change_not_invalidated(),
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
        "seam: phlow-mcp's McpServer::initialize — the real server-side \
         version negotiation (crates/phlow-mcp/src/server.rs)"
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
        "finding: known-version negotiation is correct, but unknown \
         versions silently upgrade and mid-session changes do not \
         invalidate — the skew discipline the design asks for is absent"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: phlow-mcp's McpServer::initialize (crates/phlow-mcp/src/server.rs) negotiates known versions correctly — \"2025-11-25\" -> \"2025-11-25\", \"2025-06-18\" -> \"2025-06-18\" — but unknown versions are mishandled: garbage (\"banana\") and a future version (\"2999-99-99\") both silently negotiate to the NEWEST supported version (\"2025-11-25\") with no typed error, so the server cannot distinguish a future version from garbage and a client implementing only an older version is told the server speaks the newest; and a mid-session version change (second initialize) is rejected with -32600 \"Already initialized\" but the session is NOT invalidated — it continues on the old version (ping still succeeds). Diver-owned side note, flagged: diver's ai.mcp.client sends protocolVersion='2024-11-05' (older than every version phlow-mcp supports) and discards the initialize result, so the negotiated version never reaches any feature gate — skew is silently accepted on both ends. Product decisions banked for Matt (phlow-owned, not implemented on gauntlet authority): whether unknown versions should fail closed with a typed version_negotiation_failed instead of silently upgrading to newest, and whether a mid-session re-initialize should invalidate the session.".to_string(),
        evidence,
    })
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
