//! task-92: proposal scope binding and drift detection (nvim-lua driver
//! + harness).
//!
//! The design asks for an approval to authorize *exactly* the bytes it
//! reviewed: the bound unit is hash(base revision + diff bytes); any
//! drift between approval time and apply time is detected and rejected
//! with `proposal_drift` naming the differing hunks; a clean-applying
//! rebase is still caught via the base revision in the bound hash;
//! approval for P applied to Q is rejected; and a recovery path exists
//! (re-review and re-approve — fail-closed, not fail-stuck).
//!
//! Seam mapping (verified, not invented): the lua layer (diver's
//! `ai.*`) has NO approval→content-hash binding. What exists:
//!   * `ai.harness.policy` — tool-use approvals bound to ACTION SCOPE
//!     (rule decisions 'allow'|'deny'|'approval'; task-59's mechanism).
//!     It binds *what the tool may do*, never *which bytes were
//!     reviewed*.
//!   * `ai.harness.approval` — an async approval queue (data only: id,
//!     tool, risk, summary, argv, paths; no render, no hash field).
//!   * `ai.harness.store` — a content-addressed ARTIFACT store with a
//!     `content_hash` helper. The red herring: it hashes artifact bytes
//!     for deduplication; `approval.lua` never references it, and no
//!     approval is bound to any hash.
//!
//! Absent: any `proposal_drift` typed error, any drift-detection
//! module, any re-review/re-approve path, any binding of an approval
//! id to hash(base revision + diff bytes).
//!
//! Four cases: two validation (driver probes against the REAL modules),
//! two adversarial (harness probes over the driver's machine-readable
//! traces). The task-level verdict is `fail` at `"seam"`.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//! Whether diver should bind approvals to content hashes with drift
//! detection is Matt's call. (The rust half of this design point is
//! task-91's A1: phlow-experiment's gate never compares the approval's
//! candidate digest against the proposal.)

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence, run_nvim_lua_driver_with_env};
use std::fmt;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-92";
/// Human-readable name.
pub const NAME: &str = "proposal scope binding and drift detection";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Probe cases the driver runs, in order:
/// two validation (driver probes), two adversarial (harness probes).
pub const CASES: [&str; 4] = [
    "no_content_hash_binding",
    "scope_not_hash",
    "no_drift_detection",
    "no_recovery_path",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-92 driver itself (not of the code under test).
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
                write!(f, "task-92: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-92: probe {case} failed: {detail}")
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

/// Task work dir: the nvim runner creates `ctx.work_dir/task-92`.
fn work_dir(ctx: &Ctx) -> std::path::PathBuf {
    ctx.work_dir.join("task-92")
}

/// Run one driver scenario and convert its [`TaskOutcome`] into a case
/// report.
fn run_driver_scenario(ctx: &Ctx, case: &'static str, scenario: &str) -> CaseReport {
    match run_nvim_lua_driver_with_env(
        ctx,
        "task_92.lua",
        "task-92",
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

/// V1: the approval queue exposes no content-hash binding API, and the
/// approval record carries no hash field. Over the driver's trace:
/// `binding_api` is false and `record_has_hash` is false.
fn case_no_content_hash_binding(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_content_hash_binding";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "binding_driver", "binding");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "binding-trace.json")?;
    if trace.get("scenario").and_then(serde_json::Value::as_str) != Some("binding") {
        return Ok(CaseReport::fail(
            CASE,
            "trace scenario is not 'binding'".to_string(),
            evidence,
        ));
    }
    let binding_api = trace
        .get("binding_api")
        .and_then(serde_json::Value::as_bool);
    let record_has_hash = trace
        .get("record_has_hash")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace binding_api={binding_api:?} record_has_hash={record_has_hash:?}"
    ));
    if binding_api != Some(false) || record_has_hash != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the absent binding API".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "ai.harness.approval is a data-only queue: no bind_approval / \
         proposal_hash / content_hash / candidate_digest API, and the \
         approval record carries id/tool/risk/summary/argv/paths — no \
         hash field. ai.harness.store's content_hash hashes artifact \
         bytes for dedup; the approval queue never references it"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"binding_api": false, "record_has_hash": false}),
        evidence,
    ))
}

/// V2: the policy binds approvals to action scope (rule decisions), not
/// to content hashes — the task-59 mechanism, distinct from what
/// task-92 demands. Over the driver's trace: `scope_binding` is true
/// and `hash_binding` is false.
fn case_scope_not_hash(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "scope_not_hash";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "scope_driver", "scope");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "binding-trace.json")?;
    if trace.get("scenario").and_then(serde_json::Value::as_str) != Some("scope") {
        return Ok(CaseReport::fail(
            CASE,
            "trace scenario is not 'scope'".to_string(),
            evidence,
        ));
    }
    let scope_binding = trace
        .get("scope_binding")
        .and_then(serde_json::Value::as_bool);
    let hash_binding = trace
        .get("hash_binding")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace scope_binding={scope_binding:?} hash_binding={hash_binding:?}"
    ));
    if scope_binding != Some(true) || hash_binding != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not show scope-binding without hash-binding".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "ai.harness.policy binds approvals to action scope (rule \
         decisions allow/deny/approval — the task-59 tool-use mechanism); \
         it binds what the tool may do, never which bytes were reviewed. \
         Distinct from task-92's demand: hash(base revision + diff bytes) \
         with drift detection across time"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"scope_binding": true, "hash_binding": false}),
        evidence,
    ))
}

/// A1: no drift-detection module or API exists in lua/ai. Over the
/// driver's trace: the candidate modules are all absent and the token
/// scan for `drift` finds zero hits.
fn case_no_drift_detection(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_drift_detection";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "drift_driver", "drift");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "binding-trace.json")?;
    if trace.get("scenario").and_then(serde_json::Value::as_str) != Some("drift") {
        return Ok(CaseReport::fail(
            CASE,
            "trace scenario is not 'drift'".to_string(),
            evidence,
        ));
    }
    let drift_hits = trace.get("drift_hits").and_then(serde_json::Value::as_u64);
    evidence.push(format!("trace drift_hits={drift_hits:?}"));
    if drift_hits != Some(0) {
        return Ok(CaseReport::fail(
            CASE,
            "drift vocabulary appeared in lua/ai (premise changed)".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "no ai.harness.drift / ai.self_improve / ai.approval module, and \
         zero 'drift' token hits across lua/ai: a rebase or concurrent \
         edit between approval and apply would not be detected — the \
         design's proposal_drift rejection has no implementation"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"drift_hits": 0}),
        evidence,
    ))
}

/// A2: no re-review/re-approve recovery path exists. Over the driver's
/// trace: the token scan for `re_approve`/`reapprove`/`proposal_drift`
/// finds zero hits.
fn case_no_recovery_path(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_recovery_path";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "recovery_driver", "recovery");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "binding-trace.json")?;
    if trace.get("scenario").and_then(serde_json::Value::as_str) != Some("recovery") {
        return Ok(CaseReport::fail(
            CASE,
            "trace scenario is not 'recovery'".to_string(),
            evidence,
        ));
    }
    let recovery_hits = trace
        .get("recovery_hits")
        .and_then(serde_json::Value::as_u64);
    evidence.push(format!("trace recovery_hits={recovery_hits:?}"));
    if recovery_hits != Some(0) {
        return Ok(CaseReport::fail(
            CASE,
            "recovery vocabulary appeared in lua/ai (premise changed)".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "no re-review path: fail-closed would be fail-stuck here — a \
         drifted proposal has no lua-side path back to approval"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"recovery_hits": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(ctx: &Ctx, case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "no_content_hash_binding" => case_no_content_hash_binding(ctx),
        "scope_not_hash" => case_scope_not_hash(ctx),
        "no_drift_detection" => case_no_drift_detection(ctx),
        "no_recovery_path" => case_no_recovery_path(ctx),
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
        "seam: diver's ai.harness.* — the approval queue and policy are \
         real, but neither binds approvals to content hashes: policy \
         binds to action scope (task-59), the queue is data-only, and \
         store.content_hash is artifact dedup unreachable from approvals"
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
        "finding: the lua layer has no approval→content-hash binding and \
         no drift detection — the design's hash(base revision + diff \
         bytes) bound unit has no implementation to probe"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no approval→content-hash binding exists in the lua layer. Diver's \
         ai.harness.approval is a data-only queue (no bind_approval/proposal_hash/content_hash/\
         candidate_digest API; the record carries id/tool/risk/summary/argv/paths, no hash \
         field). ai.harness.policy binds approvals to action scope via rule decisions \
         (allow/deny/approval — task-59's tool-use mechanism), never to reviewed bytes. \
         ai.harness.store's content_hash hashes artifact bytes for dedup and is never \
         referenced by the approval queue. Bounded token scans over lua/ai find zero hits for \
         `drift` and zero hits for re_approve/reapprove/proposal_drift: no drift detection, \
         no proposal_drift typed error, no re-review recovery path. A rebase or concurrent \
         edit between approval and apply would not be detected lua-side. Diver-owned finding, \
         flagged: whether diver should bind approvals to content hashes with drift detection \
         is Matt's call — never fixed on gauntlet authority. (Rust half: task-91's A1 shows \
         phlow-experiment's gate never compares the approval's candidate digest against the \
         proposal.)"
            .to_string(),
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
