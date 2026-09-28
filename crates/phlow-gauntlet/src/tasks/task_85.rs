//! task-85: provider usage accounting integrity (nvim-lua driver + harness).
//!
//! The design asks for the harness per-run cost ledger fed by
//! provider usage reports: provider-reported usage is *untrusted
//! input* — null, missing, or absurd values must not corrupt the
//! ledger. Well-formed usage is exact; missing usage is recorded as
//! an estimate with a documented method, flagged `estimated: true`;
//! absurd usage is capped at a named sanity bound and flagged;
//! corrections keep the max observed with an append-only audit note.
//!
//! Seam mapping (verified, not invented): diver's
//! `ai.harness.budget` (`lua/ai/harness/budget.lua`) is the real
//! per-run budget ledger, but it has NO usage-ingestion layer.
//! `M.new(limits)` builds the ledger; `M.check`/`M.consume(budget,
//! kind, amount)` add numbers at face value — `check` rejects only
//! negative amounts and unknown kinds; `M.snapshot()` returns bare
//! numbers. There is no estimated/measured flag, no sanity cap, no
//! correction API, no audit trail. The metrology-integrity layer
//! the design asks for has no implementation; budget enforcement
//! (task-02's consumer) eats whatever numbers were consumed.
//!
//! Four cases: two validation (driver probes against the REAL
//! module), two adversarial (harness probes over the driver's
//! machine-readable traces). The task-level verdict is `fail` at
//! `"seam"`.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//! Whether diver's budget ledger should gain usage-ingestion
//! discipline (estimated flags, sanity caps, append-only
//! corrections) is Matt's call.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence, run_nvim_lua_driver_with_env};
use std::fmt;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-85";
/// Human-readable name.
pub const NAME: &str = "provider usage accounting integrity";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Probe cases the driver runs, in order:
/// two validation (driver probes), two adversarial (harness probes).
pub const CASES: [&str; 4] = [
    "wellformed_usage_exact",
    "estimated_flag_absent",
    "absurd_usage_silent",
    "shrinking_has_no_correction",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-85 driver itself (not of the code under test).
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
                write!(f, "task-85: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-85: probe {case} failed: {detail}")
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

/// Task work dir: the nvim runner creates `ctx.work_dir/task-85`.
fn work_dir(ctx: &Ctx) -> std::path::PathBuf {
    ctx.work_dir.join("task-85")
}

/// Run one driver scenario and convert its [`TaskOutcome`] into a case
/// report.
fn run_driver_scenario(ctx: &Ctx, case: &'static str, scenario: &str) -> CaseReport {
    match run_nvim_lua_driver_with_env(
        ctx,
        "task_85.lua",
        "task-85",
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

/// A1: absurd usage is absorbed silently. Over the driver's trace:
/// `consume(b, 'token', 1e12)` returned ok=true, the snapshot shows
/// used.token == 1e12, and the trace records flagged=false,
/// capped=false.
fn case_absurd_usage_silent(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "absurd_usage_silent";
    let mut evidence = Vec::new();
    // The driver scenario must run first so the trace exists.
    let driver = run_driver_scenario(ctx, "absurd_driver", "absurd");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "usage-trace.json")?;
    let scenario = trace
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if scenario != "absurd" {
        return Ok(CaseReport::fail(
            CASE,
            format!("trace scenario is {scenario:?}, not 'absurd'"),
            evidence,
        ));
    }
    let flagged = trace.get("flagged").and_then(serde_json::Value::as_bool);
    let capped = trace.get("capped").and_then(serde_json::Value::as_bool);
    evidence.push(format!("trace flagged={flagged:?} capped={capped:?}"));
    if flagged != Some(false) || capped != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not show the absurd value absorbed unflagged/uncapped".to_string(),
            evidence,
        ));
    }
    let used = trace
        .pointer("/snapshot/used/token")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(-1.0);
    evidence.push(format!("snapshot.used.token = {used}"));
    if used != 1e12 {
        return Ok(CaseReport::fail(
            CASE,
            format!("used.token is {used}, not 1e12"),
            evidence,
        ));
    }
    evidence.push(
        "an absurd provider report (1e12 tokens) entered the ledger at face value: \
         no sanity bound, no flag — budget enforcement downstream eats it as-is"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"used_token": used, "flagged": false, "capped": false}),
        evidence,
    ))
}

/// A2: shrinking has no correction path. Over the driver's trace: the
/// module exposes exactly new/check/consume/remaining/exhausted/
/// snapshot — no correction, no audit; the trace records
/// correction_api=false, audit_trail=false.
fn case_shrinking_has_no_correction(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "shrinking_has_no_correction";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "shrinking_driver", "shrinking");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "usage-trace.json")?;
    let functions = trace
        .get("functions")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| fixture_error("usage trace", "missing functions array"))?;
    let names: Vec<&str> = functions
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    evidence.push(format!("module functions: {}", names.join(", ")));
    let want = [
        "new",
        "check",
        "consume",
        "remaining",
        "exhausted",
        "snapshot",
    ];
    for required in want {
        if !names.contains(&required) {
            return Ok(CaseReport::fail(
                CASE,
                format!("expected module function {required} missing — module changed"),
                evidence,
            ));
        }
    }
    for banned in ["correct", "audit", "revise", "adjust"] {
        if names.iter().any(|n| n.contains(banned)) {
            return Ok(CaseReport::fail(
                CASE,
                format!("unexpected correction/audit function matching {banned:?}"),
                evidence,
            ));
        }
    }
    let correction_api = trace
        .get("correction_api")
        .and_then(serde_json::Value::as_bool);
    let audit_trail = trace
        .get("audit_trail")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace correction_api={correction_api:?} audit_trail={audit_trail:?}"
    ));
    if correction_api != Some(false) || audit_trail != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the absent correction/audit path".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "a provider correction ('actually 300, not 500') has no ledger entry point: \
         the only downward path is raw table mutation, which keeps no max-observed \
         and writes no audit note — history is silently rewritten"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"correction_api": false, "audit_trail": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(ctx: &Ctx, case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "wellformed_usage_exact" => Ok(run_driver_scenario(ctx, case, "wellformed")),
        "estimated_flag_absent" => Ok(run_driver_scenario(ctx, case, "estimated")),
        "absurd_usage_silent" => case_absurd_usage_silent(ctx),
        "shrinking_has_no_correction" => case_shrinking_has_no_correction(ctx),
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
        "seam: diver's ai.harness.budget — the real per-run budget ledger, but \
         with no usage-ingestion layer: consume() takes bare numbers at face \
         value, the snapshot carries bare numbers, and there is no \
         estimated/measured flag, sanity cap, correction API, or audit trail"
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
        "finding: the ledger is exact for well-formed usage but defenseless \
         against untrusted provider reports — absurd values are absorbed \
         unflagged, estimates are indistinguishable from measurements, and \
         corrections are raw mutations with no audit"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: diver's ai.harness.budget (lua/ai/harness/budget.lua) is the real per-run budget ledger, but it has no usage-ingestion layer. Driver probes against the REAL module confirm it: well-formed usage is exact (consume(100,'token') -> snapshot.used.token == 100); consume's declaration is read from the real budget.lua source via debug.getinfo('S') — exactly (budget, kind, amount), no estimated flag parameter — and the snapshot carries bare numbers (estimated vs measured indistinguishable); consume(b,'token',1e12) returns ok with no cap and no flag (check only rejects negative amounts and unknown kinds) — the absurd value enters the ledger at face value; and the module exposes exactly new/check/consume/remaining/exhausted/snapshot — no correction API, no audit trail, so a provider correction is possible only by raw table mutation, which keeps no max-observed and writes no audit note. Budget enforcement consumes these numbers as-is. Diver-owned: flagged, never fixed on gauntlet authority — whether the ledger should gain usage-ingestion discipline (estimated flags, sanity caps, append-only corrections) is Matt's call.".to_string(),
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
/// Known scenarios: `"wellformed"`, `"estimated"`, `"absurd"`,
/// `"shrinking"`. Unknown names make the driver report failure.
pub fn run_scenario(ctx: &Ctx, scenario: &str) -> TaskOutcome {
    run_nvim_lua_driver_with_env(
        ctx,
        "task_85.lua",
        "task-85",
        &[("GAUNTLET_SCENARIO", scenario)],
    )
}
