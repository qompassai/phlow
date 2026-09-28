//! task-46: context pressure compaction (rust).
//!
//! Drives the real seam: [`phlow_agent::evaluate_compaction`] — SoL-Pi's
//! Online Context Compact re-expressed as an explicit cost model
//! (`crates/phlow-agent/src/solpi/context_compact.rs`). The engine takes a
//! snapshot of steps and returns a [`phlow_agent::CompactionPlan`]:
//! candidates are completed, unpinned steps in first-seen (oldest-first)
//! order, capped at 64; `compact` is true only when window pressure meets
//! the threshold AND reclaimable bytes meet the minimum AND the candidate
//! count meets the minimum. Pinned steps (system prompts, safety context)
//! and in-flight steps (`completed == false`) are never candidates.
//!
//! Honest scope: the engine decides and explains; compaction itself is the
//! caller's job. The design's "memory stays under the cap in a
//! sustained-fill test" is covered here for the engine's own output — the
//! plan is bounded by named constants no matter how much input arrives —
//! and the doc states plainly that a caller must act on the plan to keep
//! the real window under the cap. "Nothing currently referenced is ever
//! compacted away" is proven by reference check: every candidate id is
//! looked up in the input step set and asserted to be a completed,
//! unpinned step — no timing luck involved.
//!
//! Four cases, all against the real engine (no mocks): two validation,
//! two adversarial. The task-level verdict is `pass`.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_agent::{CompactStep, CompactionPolicy, evaluate_compaction};
use std::fmt;

/// Task id.
pub const ID: &str = "task-46";
/// Human-readable name.
pub const NAME: &str = "context pressure compaction";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "under_budget_nothing_compacted",
    "pressure_compacts_oldest_first_sustained",
    "critical_section_pressure_compacts_nothing",
    "adversarial_size_mix_never_candidates_active",
];

/// Window for the small-threshold fixtures, in bytes.
const WINDOW_BYTES: usize = 1_000;
/// Pressure threshold for the fixtures: compact only at half full.
const PRESSURE_THRESHOLD: f64 = 0.5;
/// Minimum reclaimable bytes for the fixtures.
const MIN_SAVINGS_BYTES: usize = 100;
/// Minimum candidate steps for the fixtures.
const MIN_CANDIDATE_STEPS: usize = 2;

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-46 driver itself (not of the engine under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A policy fixture was rejected at construction.
    Fixture {
        /// What was being built.
        what: String,
        /// The underlying error.
        detail: String,
    },
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture { what, detail } => {
                write!(f, "task-46: cannot build fixture {what}: {detail}")
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
// Fixtures
// ---------------------------------------------------------------------------

/// Opted-in policy with small, explicit thresholds so the fixtures stay
/// fast and the arithmetic is reviewable.
fn policy() -> Result<CompactionPolicy, DriverError> {
    CompactionPolicy::with_thresholds(
        WINDOW_BYTES,
        PRESSURE_THRESHOLD,
        MIN_SAVINGS_BYTES,
        MIN_CANDIDATE_STEPS,
    )
    .map_err(|e| fixture_error("policy", e))
}

/// One synthetic step. `content` is `bytes` fill bytes — the engine
/// measures `content.len()` itself, so size claims cannot be faked.
fn step(id: u64, bytes: usize, completed: bool, pinned: bool) -> CompactStep {
    CompactStep {
        id,
        content: "s".repeat(bytes),
        completed,
        pinned,
    }
}

/// Assert every candidate id resolves (by lookup in `steps`) to a
/// completed, unpinned step. This is the reference check the design's
/// pass criteria demand: in-flight and pinned steps are proven safe by
/// identity, not by timing.
fn assert_candidates_are_safe(
    case: &str,
    candidates: &[u64],
    steps: &[CompactStep],
) -> Result<(), String> {
    for id in candidates {
        let found = steps.iter().find(|s| s.id == *id);
        match found {
            Some(s) if s.completed && !s.pinned => {}
            Some(_) => {
                return Err(format!(
                    "case {case}: candidate id {id} resolves to an in-flight or pinned step — active reasoning would be torn"
                ));
            }
            None => {
                return Err(format!(
                    "case {case}: candidate id {id} resolves to no input step — dangling candidate"
                ));
            }
        }
    }
    Ok(())
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
            metrics: serde_json::Value::Null,
            evidence,
            failures: vec![failure],
        }
    }
}

/// V1: under budget, nothing compacts. Two completed candidates exist, but
/// window pressure (300/1000 = 0.3) is below the 0.5 threshold — the plan
/// must say no and name the threshold as the deciding check.
fn case_under_budget_nothing_compacted() -> Result<CaseReport, DriverError> {
    const CASE: &str = "under_budget_nothing_compacted";
    let policy = policy()?;
    let steps = vec![
        step(1, 150, true, false),
        step(2, 150, true, false),
        step(3, 60, false, false),
    ];
    let plan = evaluate_compaction(&policy, &steps).map_err(|e| fixture_error("evaluate", e))?;
    let mut evidence = vec![format!(
        "pressure {:.3} (threshold {PRESSURE_THRESHOLD}), savings {} bytes, {} candidates",
        plan.window_pressure,
        plan.savings_bytes,
        plan.candidate_ids.len()
    )];
    evidence.push(format!("reason: {}", plan.reason));
    if plan.compact {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "compact=true under budget (pressure {:.3})",
                plan.window_pressure
            ),
            evidence,
        ));
    }
    if !plan.reason.contains("below threshold") {
        return Ok(CaseReport::fail(
            CASE,
            format!("reason does not name the threshold check: {}", plan.reason),
            evidence,
        ));
    }
    if let Err(detail) = assert_candidates_are_safe(CASE, &plan.candidate_ids, &steps) {
        return Ok(CaseReport::fail(CASE, detail, evidence));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "compact": false,
            "window_pressure": plan.window_pressure,
            "candidate_count": plan.candidate_ids.len(),
        }),
        evidence,
    ))
}

/// V2: sustained fill. Rounds keep adding steps; every plan must stay
/// within the engine's named bounds (candidates <= 64, reason <= 512
/// chars) no matter how much input arrives — the module's own memory
/// discipline. In the final round, pressure is over the threshold and
/// several completed steps are eligible: the plan must compact, name the
/// oldest completed steps first, and never touch in-flight or pinned
/// steps (proven by the reference check).
fn case_pressure_compacts_oldest_first_sustained() -> Result<CaseReport, DriverError> {
    const CASE: &str = "pressure_compacts_oldest_first_sustained";
    let policy = policy()?;
    let mut steps: Vec<CompactStep> = Vec::new();
    let mut evidence = Vec::new();
    let mut max_candidates = 0usize;
    let mut max_reason_chars = 0usize;

    // Rounds 1..=10: each round adds one completed step (120 bytes), one
    // in-flight step (120 bytes), and every third round a pinned safety
    // step. The window fills steadily toward and past the threshold.
    for round in 1..=10u64 {
        steps.push(step(round * 3 - 2, 120, true, false));
        steps.push(step(round * 3 - 1, 120, false, false));
        if round % 3 == 0 {
            steps.push(step(round * 3, 120, true, true));
        }
        let plan =
            evaluate_compaction(&policy, &steps).map_err(|e| fixture_error("evaluate", e))?;
        max_candidates = max_candidates.max(plan.candidate_ids.len());
        max_reason_chars = max_reason_chars.max(plan.reason.chars().count());
        if plan.candidate_ids.len() > 64 {
            return Ok(CaseReport::fail(
                CASE,
                format!(
                    "round {round}: candidate list exceeded the named cap: {}",
                    plan.candidate_ids.len()
                ),
                evidence,
            ));
        }
        if plan.reason.chars().count() > 512 {
            return Ok(CaseReport::fail(
                CASE,
                format!(
                    "round {round}: reason exceeded the named cap: {} chars",
                    plan.reason.chars().count()
                ),
                evidence,
            ));
        }
        if let Err(detail) = assert_candidates_are_safe(CASE, &plan.candidate_ids, &steps) {
            return Ok(CaseReport::fail(CASE, detail, evidence));
        }
        evidence.push(format!(
            "round {round}: steps={}, pressure={:.3}, compact={}, candidates={}",
            steps.len(),
            plan.window_pressure,
            plan.compact,
            plan.candidate_ids.len()
        ));
    }

    // Final round: pressure is 2760/1000 = 2.76 — far over the
    // threshold — and 10 completed unpinned steps are eligible (plus 3
    // pinned, 10 in-flight). The plan must compact and name the oldest
    // completed steps first.
    let plan = evaluate_compaction(&policy, &steps).map_err(|e| fixture_error("evaluate", e))?;
    if !plan.compact {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "final round: compact=false under pressure {:.3} with {} eligible candidates",
                plan.window_pressure,
                plan.candidate_ids.len()
            ),
            evidence,
        ));
    }
    let expected: Vec<u64> = (1..=10u64).map(|r| r * 3 - 2).collect();
    if plan.candidate_ids != expected {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "candidate order/content wrong: got {:?}, want oldest-first {:?}",
                plan.candidate_ids, expected
            ),
            evidence,
        ));
    }
    evidence.push(format!(
        "final: pressure={:.3}, compact=true, candidates={:?} (oldest first)",
        plan.window_pressure, plan.candidate_ids
    ));
    evidence.push(format!(
        "sustained-fill: max candidates {max_candidates} (cap 64), max reason {max_reason_chars} chars (cap 512) over 10 rounds"
    ));
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "final_pressure": plan.window_pressure,
            "final_candidates": plan.candidate_ids.len(),
            "max_candidates_seen": max_candidates,
            "max_reason_chars_seen": max_reason_chars,
            "oldest_first": true,
        }),
        evidence,
    ))
}

/// A1: pressure DURING active reasoning. The window is over-full
/// (1500/1000 = 1.5) but every step is in-flight — active reasoning the
/// engine must not tear. The engine has no separate critical-section
/// flag; the protection is the completed-only eligibility rule: active
/// (in-flight) reasoning is never named a candidate, so with zero
/// completed steps the plan refuses (waits), with an empty candidate
/// list. This is the design's "compact only safe candidates or wait"
/// requirement, implemented as eligibility, not as a veto.
fn case_critical_section_pressure_compacts_nothing() -> Result<CaseReport, DriverError> {
    const CASE: &str = "critical_section_pressure_compacts_nothing";
    let policy = policy()?;
    let steps = vec![
        step(1, 500, false, false),
        step(2, 500, false, false),
        step(3, 500, false, false),
    ];
    let plan = evaluate_compaction(&policy, &steps).map_err(|e| fixture_error("evaluate", e))?;
    let mut evidence = vec![format!(
        "all steps in-flight: pressure {:.3}, candidates {:?}",
        plan.window_pressure, plan.candidate_ids
    )];
    evidence.push(format!("reason: {}", plan.reason));
    if plan.compact {
        return Ok(CaseReport::fail(
            CASE,
            "compact=true with zero completed steps — active reasoning would be torn".to_string(),
            evidence,
        ));
    }
    if !plan.candidate_ids.is_empty() {
        return Ok(CaseReport::fail(
            CASE,
            format!(
                "in-flight steps named as candidates: {:?}",
                plan.candidate_ids
            ),
            evidence,
        ));
    }
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "compact": false,
            "window_pressure": plan.window_pressure,
            "candidate_count": 0,
        }),
        evidence,
    ))
}

/// A2: adversarial size mix. The adversary makes the in-flight step and a
/// pinned safety step dominate the byte count, trying to pull active
/// reasoning into the candidate set. The reference check resolves every
/// candidate id back to the input set: each must be completed AND
/// unpinned, and the two big steps must be absent.
fn case_adversarial_size_mix_never_candidates_active() -> Result<CaseReport, DriverError> {
    const CASE: &str = "adversarial_size_mix_never_candidates_active";
    let policy = policy()?;
    let steps = vec![
        step(1, 600, false, false), // huge in-flight step: the weapon
        step(2, 300, true, true),   // huge pinned safety context: off-limits
        step(3, 80, true, false),   // small completed: eligible
        step(4, 80, true, false),   // small completed: eligible
    ];
    let plan = evaluate_compaction(&policy, &steps).map_err(|e| fixture_error("evaluate", e))?;
    let mut evidence = vec![format!(
        "pressure {:.3}, compact={}, candidates {:?}",
        plan.window_pressure, plan.compact, plan.candidate_ids
    )];
    if plan.candidate_ids.contains(&1) {
        return Ok(CaseReport::fail(
            CASE,
            "in-flight step 1 (600 bytes) named as a compaction candidate".to_string(),
            evidence,
        ));
    }
    if plan.candidate_ids.contains(&2) {
        return Ok(CaseReport::fail(
            CASE,
            "pinned step 2 (300 bytes) named as a compaction candidate".to_string(),
            evidence,
        ));
    }
    if let Err(detail) = assert_candidates_are_safe(CASE, &plan.candidate_ids, &steps) {
        return Ok(CaseReport::fail(CASE, detail, evidence));
    }
    evidence.push(
        "every candidate id resolves by lookup to a completed, unpinned step — proven by reference, not timing"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({
            "compact": plan.compact,
            "window_pressure": plan.window_pressure,
            "candidate_ids": plan.candidate_ids,
            "inflight_excluded": true,
            "pinned_excluded": true,
        }),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "under_budget_nothing_compacted" => case_under_budget_nothing_compacted(),
        "pressure_compacts_oldest_first_sustained" => {
            case_pressure_compacts_oldest_first_sustained()
        }
        "critical_section_pressure_compacts_nothing" => {
            case_critical_section_pressure_compacts_nothing()
        }
        "adversarial_size_mix_never_candidates_active" => {
            case_adversarial_size_mix_never_candidates_active()
        }
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
        "recon: the seam is phlow_agent::evaluate_compaction (crates/phlow-agent/src/solpi/context_compact.rs) — SoL-Pi Online Context Compact as an explicit cost model; candidates are completed, unpinned steps in first-seen order, capped at 64".to_string(),
        "scope: the engine decides and explains; compaction itself is the caller's job — the sustained-fill case covers the engine's bounded output under load, not the caller's window".to_string(),
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
"finding: under budget the plan refuses (compact=false); under pressure the oldest completed steps are named first and in-flight/pinned steps are never candidates — proven by id-lookup reference checks, including pressure during active reasoning (all steps in-flight) where the plan waits with an empty candidate list. The engine's protection is completed-only eligibility, not a separate critical-section flag".to_string(),
    );
    Ok(evidence)
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
