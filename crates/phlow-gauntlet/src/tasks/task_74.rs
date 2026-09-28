//! task-74: result aggregation under partial failure (rust).
//!
//! The design asks for aggregation when some contributors never
//! return: the verdict must account for every gap explicitly — never
//! silently average over the survivors. Scenarios: default (all 5
//! subagents return — aggregated normally); partial (2 of 5 time out —
//! the verdict lists exactly which contributions are missing and why);
//! adversarial-in-V: all subagents fail — a typed
//! `insufficient_contributions` verdict, not an empty success;
//! adversarial-in-V: a late result arrives AFTER aggregation —
//! recorded as late, never silently merged into the published verdict.
//! Pass criteria: the verdict enumerates missing contributions with
//! causes; "3 of 5" is distinguishable from "5 of 5"; late arrivals
//! never mutate a published verdict; quorum thresholds are named
//! constants.
//!
//! Honest result: the seam cannot meet the criteria, and the finding
//! is the gap. The real aggregator is `phlow_council::CouncilReview`
//! (`crates/phlow-council/src/review.rs`): a votes-only tally over the
//! votes it is HANDED. It carries no expected-contributor set — a
//! 3-vote "3-of-5" review is `PartialEq`-identical to a 3-vote "3-of-3"
//! review, so "3 of 5" is indistinguishable from "5 of 5" downstream.
//! There is no missing-contribution accounting, no quorum constant
//! (`REVIEWERS_MAX = 8` is a capacity bound on the votes slice, not a
//! quorum of expected contributors — and it is not even re-exported in
//! the crate's public API), no typed `insufficient_contributions`
//! (zero votes fails with `CouncilError::TooManyItems { field:
//! "reviewers", max: 8 }` — a variant whose Display reads "reviewers
//! holds more than 8 items" for ZERO reviewers), and no late-arrival
//! discipline (`decision()` is a pure function of the votes slice: no
//! published-verdict boundary, no generation token, no late marker).
//!
//! Four cases: two validation, two adversarial. The task-level verdict
//! is `fail` at `"seam"`.
//!
//! Banked for Matt (product decision, NOT auto-implemented on gauntlet
//! authority): whether phlow-council should gain an aggregation
//! envelope — an expected-reviewer set, named quorum thresholds, a
//! typed `insufficient_contributions` verdict, and a late-result policy
//! (record-as-late, never merge into a published verdict). That is a
//! new product feature, not a bug fix.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_council::{CandidateId, CouncilError, CouncilReview, Decision, Vote};
use std::fmt;

// ---------------------------------------------------------------------------
// Task metadata
// ---------------------------------------------------------------------------

/// Task id.
pub const ID: &str = "task-74";
/// Human-readable name.
pub const NAME: &str = "result aggregation under partial failure";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Probe cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "all_present_aggregates",
    "partial_returns_indistinguishable",
    "all_fail_no_typed_verdict",
    "late_arrival_silently_merges",
];

// ---------------------------------------------------------------------------
// Driver errors
// ---------------------------------------------------------------------------

/// Failures of the task-74 driver itself (not of the code under test).
#[derive(Debug, Clone)]
pub enum DriverError {
    /// A fixture (review construction) was unusable.
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
                write!(f, "task-74: cannot build fixture {what}: {detail}")
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

fn candidate() -> CandidateId {
    CandidateId(7)
}

/// V1: the complete case works — 5 of 5 reviewers vote and the real
/// tally aggregates normally. This is the mechanism the partial cases
/// are measured against.
fn case_all_present_aggregates() -> Result<CaseReport, DriverError> {
    const CASE: &str = "all_present_aggregates";
    let mut evidence = Vec::new();
    let votes: [(&str, Vote); 5] = [
        ("r1", Vote::Keep),
        ("r2", Vote::Keep),
        ("r3", Vote::Revise),
        ("r4", Vote::Keep),
        ("r5", Vote::Reject),
    ];
    let review =
        CouncilReview::new(candidate(), &votes).map_err(|e| fixture_error("5-vote review", e))?;
    let decision = review.decision();
    evidence.push(format!(
        "5/5 reviewers voted (keep=3, revise=1, reject=1); decision() = {decision:?}"
    ));
    if decision != Decision::Keep {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected Decision::Keep, got {decision:?}"),
            evidence,
        ));
    }
    evidence.push(
        "the complete case aggregates per contract: keep wins on strict plurality over \
         both reject and revise"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"votes": 5, "expected": 5}),
        evidence,
    ))
}

/// V2: 2 of 5 subagents time out — only 3 votes arrive. The review
/// carries no expected-contributor set: a 3-vote "3-of-5" review is
/// identical to a 3-vote "3-of-3" review, so the gap is
/// unrepresentable and "3 of 5" is indistinguishable from "5 of 5"
/// downstream. No quorum constant exists to consult.
fn case_partial_returns_indistinguishable() -> Result<CaseReport, DriverError> {
    const CASE: &str = "partial_returns_indistinguishable";
    let mut evidence = Vec::new();
    let three_of_five: [(&str, Vote); 3] =
        [("r1", Vote::Keep), ("r2", Vote::Revise), ("r3", Vote::Keep)];
    let partial = CouncilReview::new(candidate(), &three_of_five)
        .map_err(|e| fixture_error("3-vote review", e))?;
    // The same 3 votes framed as "everyone returned".
    let three_of_three = CouncilReview::new(candidate(), &three_of_five)
        .map_err(|e| fixture_error("3-vote review (framed complete)", e))?;
    evidence.push(format!(
        "3 of 5 expected contributors voted (r4, r5 timed out); decision() = {:?}",
        partial.decision()
    ));
    if partial != three_of_three {
        return Ok(CaseReport::fail(
            CASE,
            "3-of-5 and 3-of-3 reviews differ: an expected-count IS tracked — finding refuted"
                .to_string(),
            evidence,
        ));
    }
    evidence.push(
        "CouncilReview { candidate, votes } carries no expected-contributor set: the \
         3-vote partial review == the 3-vote complete review (PartialEq) — the two \
         missing contributions (r4, r5, timed out) appear nowhere"
            .to_string(),
    );
    evidence.push(
        "no quorum constant exists: REVIEWERS_MAX = 8 bounds the votes slice (capacity), \
         it is not a quorum of expected contributors — and it is not even re-exported in \
         phlow-council's public API (mod review is private)"
            .to_string(),
    );
    evidence.push(
        "downstream consumers of decision() cannot distinguish \"3 of 5\" from \"5 of 5\": \
         the verdict never enumerates missing contributions with causes"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"votes": 3, "expected": 5, "missing_recorded": 0}),
        evidence,
    ))
}

/// A1: all subagents fail — zero votes arrive. The design demands a
/// typed `insufficient_contributions` verdict; the real code returns
/// `CouncilError::TooManyItems { field: "reviewers", max: 8 }`, whose
/// Display reads "reviewers holds more than 8 items" for ZERO
/// reviewers — not a typed insufficient-contributions verdict, and a
/// misleading name for the empty case.
fn case_all_fail_no_typed_verdict() -> Result<CaseReport, DriverError> {
    const CASE: &str = "all_fail_no_typed_verdict";
    let mut evidence = Vec::new();
    let empty: [(&str, Vote); 0] = [];
    match CouncilReview::new(candidate(), &empty) {
        Ok(_) => {
            return Ok(CaseReport::fail(
                CASE,
                "empty review constructed: an empty success — worse than the finding".to_string(),
                evidence,
            ));
        }
        Err(e) => {
            evidence.push(format!("0 votes -> Err({e:?}); Display: \"{e}\""));
            match e {
                CouncilError::TooManyItems { field, max } => {
                    evidence.push(format!(
                        "the error is TooManyItems {{ field: \"{field}\", max: {max} }} — \
                         no insufficient_contributions variant exists in CouncilError"
                    ));
                    if field != "reviewers" || max != 8 {
                        return Ok(CaseReport::fail(
                            CASE,
                            format!("unexpected TooManyItems payload: {field}/{max}"),
                            evidence,
                        ));
                    }
                }
                other => {
                    return Ok(CaseReport::fail(
                        CASE,
                        format!("unexpected error variant: {other:?}"),
                        evidence,
                    ));
                }
            }
        }
    }
    evidence.push(
        "the empty case fails closed (no empty success), but with a misnamed error: \
         \"reviewers holds more than 8 items\" for zero reviewers. The design's typed \
         insufficient_contributions verdict — distinguishable by downstream consumers \
         from any other failure — does not exist."
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"votes": 0, "typed_insufficient_contributions": false}),
        evidence,
    ))
}

/// A2: a late result arrives AFTER aggregation. `decision()` is a pure
/// function of the votes slice — there is no published-verdict
/// boundary, no generation token, no late marker. Rebuilding the
/// review with the late vote silently changes the verdict (Keep ->
/// Revise here) with no audit trail: the late arrival is merged, not
/// recorded as late.
fn case_late_arrival_silently_merges() -> Result<CaseReport, DriverError> {
    const CASE: &str = "late_arrival_silently_merges";
    let mut evidence = Vec::new();
    let on_time: [(&str, Vote); 3] = [("r1", Vote::Keep), ("r2", Vote::Keep), ("r3", Vote::Reject)];
    let published =
        CouncilReview::new(candidate(), &on_time).map_err(|e| fixture_error("3-vote review", e))?;
    let first = published.decision();
    evidence.push(format!(
        "aggregated 3 on-time votes -> published verdict {first:?}"
    ));
    if first != Decision::Keep {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected first verdict Keep, got {first:?}"),
            evidence,
        ));
    }
    // r4's vote arrives late, after the verdict was published.
    let with_late: [(&str, Vote); 4] = [
        ("r1", Vote::Keep),
        ("r2", Vote::Keep),
        ("r3", Vote::Reject),
        ("r4", Vote::Reject),
    ];
    let rebuilt = CouncilReview::new(candidate(), &with_late)
        .map_err(|e| fixture_error("4-vote review", e))?;
    let second = rebuilt.decision();
    evidence.push(format!(
        "late vote (r4 = Reject) arrives after aggregation; rebuilt review -> {second:?}"
    ));
    if second != Decision::Revise {
        return Ok(CaseReport::fail(
            CASE,
            format!("expected rebuilt verdict Revise, got {second:?}"),
            evidence,
        ));
    }
    evidence.push(
        "the published verdict moved Keep -> Revise with no late marker, no generation \
         token, no audit trail: CouncilReview cannot represent \"recorded as late, never \
         merged\" — decision() is a pure function of the votes slice, so any caller that \
         rebuilds silently merges"
            .to_string(),
    );
    Ok(CaseReport::pass(
        CASE,
        serde_json::json!({"first": "Keep", "second": "Revise", "late_marker": false}),
        evidence,
    ))
}

/// Run one case by name.
pub fn run_case(case: &str) -> Result<CaseReport, DriverError> {
    match case {
        "all_present_aggregates" => case_all_present_aggregates(),
        "partial_returns_indistinguishable" => case_partial_returns_indistinguishable(),
        "all_fail_no_typed_verdict" => case_all_fail_no_typed_verdict(),
        "late_arrival_silently_merges" => case_late_arrival_silently_merges(),
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
        "recon: the real aggregator is phlow_council::CouncilReview (crates/phlow-council/src/review.rs) — a votes-only tally over the votes it is handed; the struct is { candidate, votes } with no expected-contributor set".to_string(),
        "recon: no quorum constant exists — REVIEWERS_MAX = 8 is a capacity bound on the votes slice, not a quorum of expected contributors (and mod review is private, so it is not even re-exported)".to_string(),
        "recon: no insufficient_contributions variant exists in CouncilError; zero votes fails with TooManyItems { field: \"reviewers\", max: 8 }".to_string(),
        "recon: decision() is a pure function of the votes slice — no published-verdict boundary, no generation token, no late marker".to_string(),
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
        "finding: CouncilReview cannot account for gaps — \"3 of 5\" is PartialEq-identical to \"3 of 3\" (missing contributions appear nowhere), the all-failed case yields a misnamed TooManyItems instead of a typed insufficient_contributions, and a late vote silently moves a published Keep to Revise with no audit trail".to_string(),
    );
    Err(TaskFailure {
        where_: "seam".to_string(),
        how: "seam cannot meet the criteria: phlow-council's CouncilReview is a votes-only tally with no expected-contributor set (a 3-vote partial review is PartialEq-identical to a 3-vote complete review, so \"3 of 5\" is indistinguishable from \"5 of 5\" downstream and the verdict never enumerates missing contributions with causes); no quorum thresholds exist (REVIEWERS_MAX = 8 is a capacity bound on the votes slice, not a quorum); zero votes fails with the misnamed CouncilError::TooManyItems { field: \"reviewers\", max: 8 } instead of a typed insufficient_contributions verdict; and decision() is a pure function of the votes slice with no published-verdict boundary — a late vote silently moves a published Keep to Revise with no late marker. Whether phlow-council should gain an aggregation envelope (expected reviewers, named quorum thresholds, typed insufficient_contributions, late-result policy) is a product decision for Matt, not a gauntlet-authorized change.".to_string(),
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
