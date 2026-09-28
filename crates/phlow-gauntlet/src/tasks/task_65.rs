//! task-65: plan cost estimation (rust).
//!
//! Recon probe: the design asks for the planner's COST MODEL — the
//! planner *estimates* budget before executing. Scenarios: default
//! (estimate within 2x of actual); adversarial: estimate exceeds the
//! run's budget (the planner refuses to emit the plan — or flags it
//! `over_budget` — rather than starting doomed work); adversarial:
//! actual cost diverges mid-run (re-estimation triggers replan-or-abort
//! per a named policy). Pass criteria: no plan starts execution without
//! an estimate on record; the estimate's units match the enforced
//! budget's units (the classic estimation/enforcement mismatch, checked
//! explicitly). Distinct from tasks 02/14 (budget *enforcement* fails
//! closed) — this is *estimation* before enforcement.
//!
//! Honest result: the seam is ABSENT, and the finding is the gap —
//! exactly what the design allows ("the planner's cost model (locate;
//! if none, the finding is the gap)"). The cost-estimation vocabulary
//! scan (`estimate`, `estimator`, `estimated`, `over_budget`,
//! `cost_model`, `pre_execution`) finds exactly one hit workspace-wide:
//! `footprint_estimate` in `phlow-inference/src/kv_policy.rs` — KV-cache
//! footprint sizing for one token, classified UNRELATED (a control
//! sample: estimation vocabulary, but not plan-cost estimation).
//!
//! What DOES exist is budget *enforcement*: the real
//! [`phlow_experiment::BudgetTracker`] (tool-call count, output-byte
//! cap, absolute deadline; checked arithmetic, fails closed) and the
//! real [`phlow_experiment::Evaluator`] stage machine
//! (Validate -> Prepare -> Execute -> Verify -> Review -> Promote). The
//! driver demonstrates the gap behaviorally: a "plan" whose declared
//! cost (100 tool calls) exceeds the budget (2 tool calls) passes
//! `validate()` — no estimate gate refuses the doomed work — and only
//! fails when `execute()` calls `consume()`. Enforcement works; the
//! estimate that would have prevented starting does not exist. The
//! estimate/enforcement unit-match check is therefore vacuous: there is
//! no estimate whose units could mismatch the enforced
//! (tool_calls, output_bytes, deadline_ms) units.
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"` — the design's pass criteria (no plan starts
//! without an estimate on record; estimate units match enforcement
//! units) need a cost estimator, and there is none.
//!
//! Banked for Matt (product decision, NOT auto-implemented): whether
//! phlow wants pre-execution cost estimation at all; what its units
//! would be (tool calls? output bytes? ms?); whether an over-budget
//! estimate refuses the plan or flags it; and what policy governs
//! estimate-vs-actual divergence mid-run (replan-or-abort).

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_experiment::{BudgetTracker, EvalStage, Evaluator};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-65";
/// Human-readable name.
pub const NAME: &str = "plan cost estimation";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "no_plan_cost_estimator",
    "enforcement_without_estimation",
    "doomed_work_starts_at_enforcement",
    "units_match_check_vacuous",
];

/// Largest Rust source file the probe will scan, in bytes.
const SOURCE_BYTES_MAX: usize = 1_048_576;
/// Most source files the probe will scan before stopping.
const SOURCE_FILES_MAX: usize = 50_000;

/// Cost-estimation vocabulary: what a plan-cost estimator would be
/// named. Exact alphanumeric tokens, case-insensitive.
const ESTIMATE_TOKENS: [&str; 6] = [
    "estimate",
    "estimator",
    "estimated",
    "over_budget",
    "cost_model",
    "pre_execution",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-65 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture (workspace root, source tree, evaluator) was unusable.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
    /// A probe step failed.
    Probe {
        /// What was being probed.
        what: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-65: cannot build fixture {what}: {detail}")
            }
            Self::Probe { what, detail } => {
                write!(f, "task-65: cannot probe {what}: {detail}")
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

fn probe_error(what: &str, detail: impl fmt::Display) -> DriverError {
    DriverError::Probe {
        what: what.to_string(),
        detail: detail.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Case report
// ---------------------------------------------------------------------------

/// One probe case's outcome.
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
            metrics: serde_json::Value::Null,
            evidence,
            failures: vec![failure],
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures: the working tree and the real evaluator are the task source
// ---------------------------------------------------------------------------

/// Workspace root: two levels above this crate's manifest directory.
/// The probe reads the live working tree, never a cached copy.
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

/// Walk `crates/` under the workspace root and return `path:line` hits
/// for `.rs` files inside a `src` tree whose alphanumeric-token stream
/// contains `token` (case-insensitive, exact token — not a substring).
/// Skips the entire phlow-gauntlet crate: the gauntlet's own driver
/// docs legitimately carry estimation vocabulary, and they are test
/// scaffolding, not the product surface under test. Bounded: files over
/// [`SOURCE_BYTES_MAX`] are skipped, and the walk stops after
/// [`SOURCE_FILES_MAX`] files.
fn scan_sources(root: &Path, token: &str) -> Result<Vec<String>, DriverError> {
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
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.components().any(|c| c.as_os_str() == "src")
                && !path.components().any(|c| c.as_os_str() == "phlow-gauntlet")
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
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                for (lineno, line) in text.lines().enumerate() {
                    let found = line
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|tok| tok.eq_ignore_ascii_case(token));
                    if found {
                        hits.push(format!("{}:{}", path.display(), lineno + 1));
                    }
                }
            }
        }
    }
    Ok(hits)
}

/// Scan for several tokens at once, concatenating the hits.
fn scan_tokens(root: &Path, tokens: &[&str]) -> Result<Vec<String>, DriverError> {
    let mut hits = Vec::new();
    for token in tokens {
        hits.extend(scan_sources(root, token)?);
    }
    Ok(hits)
}

/// Build a real `BudgetTracker`: 10 tool calls, 1 KiB output, a far
/// deadline. Rejects only on zero inputs, which these are not.
fn make_tracker(tool_calls_max: u64) -> Result<BudgetTracker, DriverError> {
    BudgetTracker::new(tool_calls_max, 1024, 3_600_000)
        .map_err(|e| fixture_error("budget tracker", e))
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// V1: locate the plan-cost estimator — there is none. The estimation
/// vocabulary scan returns exactly one hit workspace-wide:
/// `footprint_estimate` in `phlow-inference/src/kv_policy.rs`, which
/// estimates the KV-cache footprint for ONE TOKEN — estimation
/// vocabulary, but not plan-cost estimation. Classified as the control
/// sample (UNRELATED), never counted as the seam.
fn case_no_plan_cost_estimator() -> Result<CaseReport, DriverError> {
    const CASE: &str = "no_plan_cost_estimator";
    let root = workspace_root()?;
    let mut evidence = Vec::new();
    let hits = scan_tokens(&root, &ESTIMATE_TOKENS)?;
    evidence.push(format!(
        "estimation vocabulary hits workspace-wide ({} tokens: {}): {}",
        ESTIMATE_TOKENS.len(),
        ESTIMATE_TOKENS.join(", "),
        hits.len()
    ));
    let mut estimator_hits = 0usize;
    for hit in &hits {
        if hit.contains("phlow-inference/src/kv_policy.rs") {
            evidence.push(format!(
                "classified control sample (UNRELATED): {hit} — KV-cache footprint \
                 sizing for one token, not plan-cost estimation"
            ));
        } else {
            estimator_hits += 1;
            evidence.push(format!("UNCLASSIFIED estimation hit: {hit}"));
        }
    }
    if estimator_hits > 0 {
        return Ok(CaseReport::fail(
            CASE,
            "a plan-cost estimation hit outside kv_policy.rs — probe outdated".to_string(),
            evidence,
        ));
    }
    evidence.push(
        "no plan-cost estimator exists: the only estimation vocabulary in the workspace \
         is KV-cache footprint sizing (phlow-inference), which estimates memory for one \
         token, not budget for a plan"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"total_hits": hits.len(), "estimator_hits": 0}),
        evidence,
    ))
}

/// V2: enforcement exists WITHOUT estimation. Drive the real
/// `Evaluator` through Validate -> Prepare -> Execute with an
/// in-budget declared cost and show the stage order contains no
/// Estimate stage: `stage()` walks Validate, Prepare, Execute with no
/// estimate step anywhere, and `Evaluator` exposes no `estimate`
/// method. Budget enforcement (`consume`) is real; the estimate that
/// would precede it is absent.
fn case_enforcement_without_estimation() -> Result<CaseReport, DriverError> {
    const CASE: &str = "enforcement_without_estimation";
    let mut evidence = Vec::new();
    let tracker = make_tracker(10)?;
    let mut evaluator = Evaluator::new(tracker);
    assert_eq!(evaluator.stage(), EvalStage::Validate);
    evaluator
        .validate()
        .map_err(|e| probe_error("validate", e))?;
    assert_eq!(evaluator.stage(), EvalStage::Prepare);
    evidence
        .push("validate() passed: no estimate gate refused the plan before execution".to_string());
    evaluator.prepare().map_err(|e| probe_error("prepare", e))?;
    assert_eq!(evaluator.stage(), EvalStage::Execute);
    evaluator
        .execute(3, 100)
        .map_err(|e| probe_error("execute", e))?;
    assert_eq!(evaluator.stage(), EvalStage::Verify);
    let remaining = evaluator.budget().tool_calls_remaining();
    evidence.push(format!(
        "execute(3 tool calls, 100 bytes) consumed budget: {remaining} tool calls remaining"
    ));
    evidence.push(
        "stage order is Validate -> Prepare -> Execute -> Verify -> Review -> Promote \
         (EvalStage::next): there is no Estimate stage and Evaluator exposes no estimate() \
         method — enforcement without estimation"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"tool_calls_remaining": remaining}),
        evidence,
    ))
}

/// A1: doomed work STARTS — only enforcement stops it. A "plan" whose
/// declared cost (100 tool calls) exceeds the budget (2 tool calls)
/// passes `validate()` and `prepare()` with no estimate gate refusing
/// it; `execute()` then fails closed at `consume()` with
/// `BudgetExhausted`. This is the design's adversarial scenario
/// demonstrated as the gap: without estimation, the system starts
/// doomed work and relies on enforcement to kill it.
fn case_doomed_work_starts_at_enforcement() -> Result<CaseReport, DriverError> {
    const CASE: &str = "doomed_work_starts_at_enforcement";
    let mut evidence = Vec::new();
    let tracker = make_tracker(2)?;
    let mut evaluator = Evaluator::new(tracker);
    // The doomed plan's declared cost dwarfs the budget — and there is
    // no estimate gate to refuse it before execution starts.
    evaluator
        .validate()
        .map_err(|e| probe_error("validate (doomed plan)", e))?;
    evidence.push(
        "validate() PASSED for a plan declaring 100 tool calls against a 2-call budget: \
         no pre-execution estimate refused the doomed work"
            .to_string(),
    );
    evaluator
        .prepare()
        .map_err(|e| probe_error("prepare (doomed plan)", e))?;
    match evaluator.execute(100, 0) {
        Ok(()) => {
            return Ok(CaseReport::fail(
                CASE,
                "execute(100, 0) succeeded against a 2-call budget — enforcement broken"
                    .to_string(),
                evidence,
            ));
        }
        Err(e) => {
            evidence.push(format!(
                "execute(100, 0) failed closed at enforcement: {e} — enforcement \
                 killed the doomed work that estimation would have refused to start"
            ));
        }
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"validate_passed_doomed": true, "execute_rejected": true}),
        evidence,
    ))
}

/// A2: the estimate/enforcement unit-match check is vacuous. The
/// enforced units are named by `BudgetTracker::new`: tool_calls_max
/// (count), output_bytes_max (bytes), deadline_ms (milliseconds) — and
/// `consume(tool_calls, output_bytes)` speaks the same units. There is
/// no estimate, so there is nothing whose units could mismatch the
/// enforced ones: the classic estimation/enforcement mismatch cannot
/// even be checked. Recorded as part of the gap, not as a pass.
fn case_units_match_check_vacuous() -> Result<CaseReport, DriverError> {
    const CASE: &str = "units_match_check_vacuous";
    let mut evidence = Vec::new();
    evidence.push(
        "enforced units (BudgetTracker::new): tool_calls_max in tool-call count, \
         output_bytes_max in bytes, deadline_ms in milliseconds"
            .to_string(),
    );
    evidence.push(
        "consume(tool_calls, output_bytes) speaks the same units as the budget: \
         enforcement is unit-consistent with itself"
            .to_string(),
    );
    evidence.push(
        "no estimate exists, so the estimate-vs-enforcement unit-match check is \
         inapplicable — the classic mismatch (estimate in tokens, enforcement in \
         tool calls) cannot be ruled in or out. Vacuous, recorded as part of the gap."
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"enforced_units": ["tool_calls", "output_bytes", "deadline_ms"],
                           "estimate_units": null}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "no_plan_cost_estimator" => case_no_plan_cost_estimator(),
        "enforcement_without_estimation" => case_enforcement_without_estimation(),
        "doomed_work_starts_at_enforcement" => case_doomed_work_starts_at_enforcement(),
        "units_match_check_vacuous" => case_units_match_check_vacuous(),
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
        "recon: the estimation vocabulary scan (estimate, estimator, estimated, over_budget, cost_model, pre_execution) finds no plan-cost estimator — the sole hit is footprint_estimate in phlow-inference/src/kv_policy.rs (KV-cache sizing for one token), classified UNRELATED".to_string(),
        "recon: budget ENFORCEMENT is real — BudgetTracker (tool-call count, output-byte cap, absolute deadline; checked arithmetic, fails closed) and the Evaluator stage machine (Validate -> Prepare -> Execute -> Verify -> Review -> Promote) with no Estimate stage and no estimate() method".to_string(),
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
        "finding: estimation is absent on top of working enforcement — a plan declaring 100 tool calls against a 2-call budget passes validate() and is only stopped by consume() at execute(); the estimate/enforcement unit-match check is vacuous with no estimate to compare".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam absent (the finding IS the gap, as the design allows): phlow has no plan-cost estimator — the estimation vocabulary scan (estimate, estimator, estimated, over_budget, cost_model, pre_execution) returns no plan-cost hit workspace-wide (the sole hit, footprint_estimate in phlow-inference/src/kv_policy.rs, sizes KV-cache for one token and is classified UNRELATED). Budget enforcement is real (BudgetTracker: tool-call count, output-byte cap, absolute deadline, checked arithmetic, fails closed; Evaluator stage order Validate -> Prepare -> Execute -> Verify -> Review -> Promote with no Estimate stage and no estimate() method), but the design's pass criteria need estimation BEFORE enforcement: no plan starts execution without an estimate on record, and the estimate's units matching the enforced units. Demonstrated: a doomed plan (100 declared tool calls vs a 2-call budget) passes validate() — no estimate gate refuses it — and only fails closed at consume() inside execute(). Whether phlow wants pre-execution cost estimation, in what units, with what over-budget policy, and what estimate-vs-actual divergence policy governs mid-run replan-or-abort is banked for Matt — a product decision, not a bug.".to_string(),
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
