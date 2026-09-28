//! task-88: A2A error-shape propagation (rust recon + harness).
//!
//! The design asks for the A2A client boundary to carry remote errors
//! across as *typed* errors — code, message, and data preserved —
//! while attacker-controlled error `data` is treated as untrusted
//! (size-bounded, quoted in logs, never interpreted or interpolated),
//! unknown task-state strings are rejected as protocol violations,
//! and a peer's claim about the local run is recorded as a remote
//! assertion only (the local run's own state stays authoritative).
//!
//! Seam mapping (verified, not invented): there is NO rust A2A client
//! boundary. Exact-token scans over every product crate's
//! `src/**/*.rs` (the gauntlet crate itself excluded, per the
//! harness-probe principle) find zero hits for `a2a`, for
//! task-state vocabulary, for agent-card handling, and for the A2A
//! JSON-RPC methods (`tasks/send`, `message/send`, `tasks/get`,
//! `tasks/cancel`). The only A2A boundary in the workspace is
//! diver's Lua (`lua/ai/a2a/client.lua` + `lua/ai/a2a/tasks.lua`,
//! exercised by task-07) — and per task-07's `bad-state` scenario,
//! that boundary IGNORES unknown state strings in the task state
//! machine rather than rejecting them as protocol violations
//! (diver-owned observation, flagged in the report).
//!
//! Four cases: two validation, two adversarial — each a bounded
//! source recon that fails closed (premise changed) if A2A
//! vocabulary ever appears in a product crate. The task-level
//! verdict is `fail` at `"seam"`: the seam is absent.
//!
//! Product-decision bank: whether phlow should gain a rust A2A
//! client boundary with typed error-shape propagation and
//! untrusted-data discipline is Matt's call — not implemented on
//! gauntlet authority.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-88";
/// Human-readable name.
pub const NAME: &str = "A2A error-shape propagation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_a2a_client_boundary",
    "no_task_state_machine",
    "no_agent_card_handling",
    "no_a2a_rpc_methods",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-88 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// A recon probe failed.
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
                write!(f, "task-88: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-88: probe {case} failed: {detail}")
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

fn probe_error(case: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Probe {
        case: case.to_string(),
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
// Source probe (task_48 scan pattern)
// ---------------------------------------------------------------------------

/// Maximum source files the probe may read.
const SOURCE_FILES_MAX: usize = 4000;
/// Maximum bytes per source file the probe reads.
const SOURCE_BYTES_MAX: usize = 512 * 1024;

/// Workspace root: two levels above this crate's manifest directory.
fn workspace_root() -> Result<PathBuf, DriverError> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| fixture_error("workspace root", "manifest dir has no grandparent"))?;
    if !root.join("Cargo.lock").is_file() {
        return Err(fixture_error(
            "workspace root",
            format!("no Cargo.lock under {}", root.display()),
        ));
    }
    Ok(root.to_path_buf())
}

/// The gauntlet's own crate root, excluded from the product scan: the
/// harness's own probes use the design vocabulary.
fn excluded_crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Exact-token (case-insensitive) hits for `token` over every product
/// crate's `src/**/*.rs` — the gauntlet crate itself excluded. Bounded
/// like task_48.
fn scan_workspace(root: &Path, token: &str) -> Result<Vec<String>, DriverError> {
    let excluded = excluded_crate_root();
    let wanted = token.to_lowercase();
    let crates_dir = root.join("crates");
    let mut hits = Vec::new();
    let mut files_seen = 0usize;
    let mut stack = vec![crates_dir];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| fixture_error("source walk", format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| fixture_error("source walk", e))?;
            let path = entry.path();
            if path.is_dir() {
                if path != excluded {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.components().any(|c| c.as_os_str() == "src")
            {
                files_seen += 1;
                if files_seen > SOURCE_FILES_MAX {
                    return Err(probe_error(
                        "source scan",
                        format!("file budget {SOURCE_FILES_MAX} exhausted"),
                    ));
                }
                let bytes = std::fs::read(&path).map_err(|e| {
                    fixture_error("source read", format!("{}: {e}", path.display()))
                })?;
                if bytes.len() > SOURCE_BYTES_MAX {
                    return Err(probe_error(
                        "source scan",
                        format!("{} exceeds {SOURCE_BYTES_MAX} bytes", path.display()),
                    ));
                }
                let text = String::from_utf8_lossy(&bytes).to_lowercase();
                // Tokenize on non-alphanumeric boundaries (keep `_`, `/`,
                // `-` inside tokens so `tasks/send` matches whole).
                let mut token_start: Option<usize> = None;
                let mut found = false;
                for (idx, ch) in text.char_indices() {
                    let is_token = ch.is_alphanumeric() || ch == '_' || ch == '/' || ch == '-';
                    if is_token {
                        if token_start.is_none() {
                            token_start = Some(idx);
                        }
                    } else if let Some(start) = token_start.take()
                        && text[start..idx] == wanted
                    {
                        found = true;
                        break;
                    }
                }
                if !found
                    && let Some(start) = token_start
                    && text[start..] == wanted
                {
                    found = true;
                }
                if found {
                    hits.push(path.display().to_string());
                }
            }
        }
    }
    Ok(hits)
}

/// Assert an exact-token scan finds zero hits; fail closed (premise
/// changed) when the absence finding is refuted.
fn assert_absent(
    case: &'static str,
    root: &Path,
    tokens: &[&str],
) -> Result<CaseReport, DriverError> {
    let mut evidence = Vec::new();
    let mut total_hits = 0usize;
    for token in tokens {
        let hits = scan_workspace(root, token)?;
        evidence.push(format!(
            "exact-token scan for '{token}' over crates/*/src/**/*.rs: {} hit(s)",
            hits.len()
        ));
        for hit in &hits {
            evidence.push(format!("  unexpected hit: {hit}"));
        }
        total_hits += hits.len();
    }
    if total_hits != 0 {
        return Ok(CaseReport::fail(
            case,
            format!("{total_hits} hit(s) — the absence finding is refuted (premise changed)"),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        case,
        serde_json::json!({"hits": 0}),
        evidence,
    ))
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: no A2A client boundary exists in any rust product crate.
fn case_no_a2a_client_boundary() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_a2a_client_boundary";
    let root = workspace_root()?;
    let mut report = assert_absent(CASE, &root, &["a2a"])?;
    report.evidence.push(
        "no rust crate names an a2a module, type, or function: the A2A \
         client boundary the design asks about has no rust implementation"
            .to_string(),
    );
    Ok(report)
}

/// V2: no task-state machine exists in rust — there is nothing that
/// could reject an unknown state string as a protocol violation.
fn case_no_task_state_machine() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_task_state_machine";
    let root = workspace_root()?;
    let mut report = assert_absent(CASE, &root, &["task_state", "taskstate"])?;
    report.evidence.push(
        "no rust TaskState enum or task_state field: unknown task-state \
         strings cannot be rejected because no state machine exists to \
         reject them"
            .to_string(),
    );
    Ok(report)
}

/// A1: no agent-card handling exists in rust — version/capability
/// fields from a card have no rust reader, so a peer's card claims
/// cross no rust boundary at all.
fn case_no_agent_card_handling() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_agent_card_handling";
    let root = workspace_root()?;
    let mut report = assert_absent(CASE, &root, &["agent_card", "agentcard"])?;
    report.evidence.push(
        "no rust code reads an A2A agent card: card-driven error and \
         version claims never reach rust"
            .to_string(),
    );
    Ok(report)
}

/// A2: none of the A2A JSON-RPC methods exist in rust — error
/// responses to tasks/send, message/stream, tasks/get, or
/// tasks/cancel have no rust handler that could preserve (or
/// mishandle) their shape, and attacker-controlled error `data`
/// crosses no rust boundary.
fn case_no_a2a_rpc_methods() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_a2a_rpc_methods";
    let root = workspace_root()?;
    let mut report = assert_absent(
        CASE,
        &root,
        &["tasks/send", "message/send", "tasks/get", "tasks/cancel"],
    )?;
    report.evidence.push(
        "no rust handler speaks the A2A methods: remote error shapes — \
         code, message, attacker-controlled data — cross no rust \
         boundary, so there is nothing to preserve or bound"
            .to_string(),
    );
    Ok(report)
}

/// Run one case by name.
pub fn run_case(case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "no_a2a_client_boundary" => case_no_a2a_client_boundary(),
        "no_task_state_machine" => case_no_task_state_machine(),
        "no_agent_card_handling" => case_no_agent_card_handling(),
        "no_a2a_rpc_methods" => case_no_a2a_rpc_methods(),
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
        "seam: ABSENT in rust — no A2A client boundary exists in any \
         phlow rust crate; the only A2A boundary in the workspace is \
         diver's Lua (ai.a2a.client / ai.a2a.tasks, exercised by task-07)"
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
        "finding: the rust seam is absent — typed error-shape \
         propagation for A2A has no implementation to probe"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no rust A2A client boundary exists. Bounded exact-token scans over every product crate's src/**/*.rs (gauntlet crate excluded per the harness-probe principle) find zero hits for `a2a`, for task-state vocabulary (`task_state`, `taskstate`), for agent-card handling (`agent_card`, `agentcard`), and for the A2A JSON-RPC methods (`tasks/send`, `message/send`, `tasks/get`, `tasks/cancel`). Remote error shapes — code, message, attacker-controlled data — cross no rust boundary, so there is nothing to preserve, bound, or quote. The only A2A boundary in the workspace is diver's Lua (lua/ai/a2a/client.lua, lua/ai/a2a/tasks.lua; task-07's happy path), where the `bad-state` scenario showed unknown state strings are IGNORED by the task state machine rather than rejected as protocol violations — diver-owned, flagged. Product decision banked for Matt: whether phlow should gain a rust A2A client boundary with typed error-shape propagation (code/message/data preserved; attacker data size-bounded, quoted, never interpolated; unknown states rejected; remote claims about the local run recorded as remote assertions only) is not implemented on gauntlet authority.".to_string(),
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
