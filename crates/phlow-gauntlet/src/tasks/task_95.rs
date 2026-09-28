//! task-95: approval render integrity (WYSIWYG) (nvim-lua driver +
//! harness).
//!
//! The design asks for the approval UI to render the diff from the
//! exact bytes the approval will bind to (what you see is what you
//! sign): terminal escape sequences in diffs are stripped/neutralized
//! with a warning; every elision gets a visible marker while the
//! approval binds the full bytes regardless of what's shown; and a
//! semantic-change summary (added/removed identifiers, changed control
//! flow) is always shown beside the author's rationale.
//!
//! Seam mapping (verified, not invented): the lua layer has NO proposal
//! approval render UI. `ai.harness.approval` is a data-only queue
//! (id, tool, risk, summary, argv, paths, endpoints; state
//! pending/approved/denied/expired) — its own header says "The single
//! approval surface renders from this queue", but no render function
//! exists in lua/ai: the surface lives outside the lua tree. No module
//! renders a proposal diff, neutralizes terminal escapes, marks
//! elisions, or builds a semantic-change summary. The approval record
//! carries a free-text `summary`, never diff bytes, so there is nothing
//! for a WYSIWYG check to bind to.
//!
//! Four cases: two validation (driver probes against the REAL module),
//! two adversarial (harness probes over the driver's machine-readable
//! traces). The task-level verdict is `fail` at `"seam"`.
//!
//! Diver-owned finding: flagged, never fixed on gauntlet authority.
//! Whether diver should render approval diffs with WYSIWYG integrity is
//! Matt's call.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence, run_nvim_lua_driver_with_env};
use std::fmt;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-95";
/// Human-readable name.
pub const NAME: &str = "approval render integrity (WYSIWYG)";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::NvimLua;

/// Probe cases the driver runs, in order:
/// two validation (driver probes), two adversarial (harness probes).
pub const CASES: [&str; 4] = [
    "no_render_api",
    "no_diff_render_pipeline",
    "no_escape_neutralization",
    "no_elision_marking",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-95 driver itself (not of the code under test).
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
                write!(f, "task-95: cannot build fixture {what}: {detail}")
            }
            Self::Probe { case, detail } => {
                write!(f, "task-95: probe {case} failed: {detail}")
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

/// Task work dir: the nvim runner creates `ctx.work_dir/task-95`.
fn work_dir(ctx: &Ctx) -> std::path::PathBuf {
    ctx.work_dir.join("task-95")
}

/// Run one driver scenario and convert its [`TaskOutcome`] into a case
/// report.
fn run_driver_scenario(ctx: &Ctx, case: &'static str, scenario: &str) -> CaseReport {
    match run_nvim_lua_driver_with_env(
        ctx,
        "task_95.lua",
        "task-95",
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
            format!("driver scenario failed at {where_}: {how}"),
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

/// V1: the approval queue is data-only — no render API — and the
/// approval record carries no diff bytes. Over the driver's trace:
/// `render_api` is false and `record_has_diff` is false.
fn case_no_render_api(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_render_api";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "ui_driver", "ui");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "render-trace.json")?;
    if trace.get("scenario").and_then(serde_json::Value::as_str) != Some("ui") {
        return Ok(CaseReport::fail(
            CASE,
            "trace scenario is not 'ui'".to_string(),
            evidence,
        ));
    }
    let render_api = trace.get("render_api").and_then(serde_json::Value::as_bool);
    let record_has_diff = trace
        .get("record_has_diff")
        .and_then(serde_json::Value::as_bool);
    evidence.push(format!(
        "trace render_api={render_api:?} record_has_diff={record_has_diff:?}"
    ));
    if render_api != Some(false) || record_has_diff != Some(false) {
        return Ok(CaseReport::fail(
            CASE,
            "trace does not confirm the absent render API".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "ai.harness.approval exposes no render/show/display/format_diff \
         API, and the approval record carries summary text only — no \
         diff bytes. Its header says the single approval surface renders \
         from this queue, but no render function exists in lua/ai: the \
         surface lives outside the lua tree"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"render_api": false, "record_has_diff": false}),
        evidence,
    ))
}

/// V2: no diff-render pipeline exists in the approval path anywhere in
/// lua/ai. Over the driver's trace: the token scan for
/// diff_render/render_diff/approval_ui finds zero hits.
fn case_no_diff_render_pipeline(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_diff_render_pipeline";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "diff_driver", "diff");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "render-trace.json")?;
    if trace.get("scenario").and_then(serde_json::Value::as_str) != Some("diff") {
        return Ok(CaseReport::fail(
            CASE,
            "trace scenario is not 'diff'".to_string(),
            evidence,
        ));
    }
    let render_hits = trace.get("render_hits").and_then(serde_json::Value::as_u64);
    evidence.push(format!("trace render_hits={render_hits:?}"));
    if render_hits != Some(0) {
        return Ok(CaseReport::fail(
            CASE,
            "render vocabulary appeared in lua/ai (premise changed)".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "zero diff_render/render_diff/approval_ui token hits across \
         lua/ai: no module renders a proposal diff for approval"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"render_hits": 0}),
        evidence,
    ))
}

/// A1: no escape-neutralization exists — there is no renderer whose
/// output could hide lines or rewrite displayed text. Over the
/// driver's trace: the token scan for
/// strip_escapes/neutralize/sanitize_render finds zero hits.
fn case_no_escape_neutralization(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_escape_neutralization";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "escapes_driver", "escapes");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "render-trace.json")?;
    if trace.get("scenario").and_then(serde_json::Value::as_str) != Some("escapes") {
        return Ok(CaseReport::fail(
            CASE,
            "trace scenario is not 'escapes'".to_string(),
            evidence,
        ));
    }
    let escape_hits = trace.get("escape_hits").and_then(serde_json::Value::as_u64);
    evidence.push(format!("trace escape_hits={escape_hits:?}"));
    if escape_hits != Some(0) {
        return Ok(CaseReport::fail(
            CASE,
            "escape-handling vocabulary appeared in lua/ai (premise changed)".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "zero strip_escapes/neutralize/sanitize_render token hits across \
         lua/ai: with no renderer, an ANSI-bomb in a diff is out of \
         scope — there is no render surface to harden"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"escape_hits": 0}),
        evidence,
    ))
}

/// A2: no elision marking exists — nothing collapses hunks, so no
/// marker discipline can be verified. Over the driver's trace: the
/// token scan for elision/collapsed_hunk/hunk_marker finds zero hits.
fn case_no_elision_marking(ctx: &Ctx) -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_elision_marking";
    let mut evidence = Vec::new();
    let driver = run_driver_scenario(ctx, "elision_driver", "elision");
    if !driver.passed {
        return Ok(CaseReport::fail(
            CASE,
            format!("driver scenario failed: {}", driver.failures.join("; ")),
            driver.evidence,
        ));
    }
    let trace = read_trace(ctx, "render-trace.json")?;
    if trace.get("scenario").and_then(serde_json::Value::as_str) != Some("elision") {
        return Ok(CaseReport::fail(
            CASE,
            "trace scenario is not 'elision'".to_string(),
            evidence,
        ));
    }
    let elision_hits = trace
        .get("elision_hits")
        .and_then(serde_json::Value::as_u64);
    evidence.push(format!("trace elision_hits={elision_hits:?}"));
    if elision_hits != Some(0) {
        return Ok(CaseReport::fail(
            CASE,
            "elision vocabulary appeared in lua/ai (premise changed)".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "zero elision/collapsed_hunk/hunk_marker token hits across \
         lua/ai: nothing collapses hunks, so no marker discipline can \
         be verified — and the approval record carries no diff bytes \
         for a WYSIWYG check to bind"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"elision_hits": 0}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(ctx: &Ctx, case: &'static str) -> Result<CaseReport, DriverError> {
    match case {
        "no_render_api" => case_no_render_api(ctx),
        "no_diff_render_pipeline" => case_no_diff_render_pipeline(ctx),
        "no_escape_neutralization" => case_no_escape_neutralization(ctx),
        "no_elision_marking" => case_no_elision_marking(ctx),
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
        "seam: diver's ai.harness.approval — a real, data-only queue \
         whose header promises 'the single approval surface renders from \
         this queue', but no render function exists in the lua tree"
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
        "finding: the lua layer has no proposal approval render UI — the \
         design's render-from-the-exact-bytes requirement has no \
         implementation to probe"
            .to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent: no proposal approval render UI exists in the lua layer. Diver's \
         ai.harness.approval is a data-only queue exposing no render/show/display/format_diff \
         API; the approval record carries a free-text `summary`, never diff bytes, so there is \
         nothing for a WYSIWYG check to bind to. Its header says 'The single approval surface \
         renders from this queue', but no render function exists in lua/ai — the surface lives \
         outside the lua tree. Bounded token scans over lua/ai find zero hits for \
         diff_render/render_diff/approval_ui (no diff-render pipeline), zero hits for \
         strip_escapes/neutralize/sanitize_render (no escape neutralization), and zero hits for \
         elision/collapsed_hunk/hunk_marker (no elision marking). With no renderer, an \
         ANSI-bomb diff or a hidden-elision diff is out of scope: there is no render surface to \
         harden. Diver-owned finding, flagged: whether diver should render approval diffs from \
         the exact bytes the approval binds to, with escape neutralization, elision markers, \
         and a semantic-change summary, is Matt's call — never fixed on gauntlet authority."
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
