//! Integration tests for task-74 (result aggregation under partial failure).
//!
//! The seam is REAL but does not meet the criteria: phlow's
//! aggregator `phlow_council::CouncilReview` is a votes-only tally over
//! the votes it is handed — `{ candidate, votes }` with no
//! expected-contributor set, so "3 of 5" is `PartialEq`-identical to
//! "3 of 3" and the verdict never enumerates missing contributions
//! with causes; zero votes fails with the misnamed
//! `CouncilError::TooManyItems { field: "reviewers", max: 8 }` (no
//! typed `insufficient_contributions`); and `decision()` is a pure
//! function of the votes slice with no published-verdict boundary, so
//! a late vote silently moves a published Keep to Revise with no audit
//! trail. The task-level verdict is `fail` at `"seam"`.
//!
//! Whether phlow-council should gain an aggregation envelope
//! (expected reviewers, named quorum thresholds, typed
//! insufficient_contributions, a late-result policy) is a product
//! decision for Matt — banked, not implemented on gauntlet authority.
//!
//! Four cases — 2 validation, 2 adversarial — each self-checking:
//! cases probe the seam and record measured mechanism evidence; the
//! driver then reports the honest seam failure.

use phlow_gauntlet::tasks::task_74;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

// --- validation ---

/// V1: the complete case — 5 of 5 reviewers vote; the real tally
/// aggregates normally (strict plurality: keep=3 > reject=1, revise=1
/// -> Keep). This is the mechanism baseline the partial cases are
/// measured against.
#[test]
fn complete_case_aggregates_normally() {
    assert_eq!(task_74::ID, "task-74");
    assert_eq!(task_74::NAME, "result aggregation under partial failure");
    assert_eq!(task_74::KIND, TaskKind::Rust);
    assert_eq!(task_74::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_74::run_case("all_present_aggregates")
        .unwrap_or_else(|e| panic!("task-74 case failed to run: {e}"));
    assert!(
        report.passed,
        "complete case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["votes"], 5);
    assert_eq!(report.metrics["expected"], 5);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("decision() = Keep"),
        "evidence must show the aggregated verdict:\n{joined}"
    );
}

/// V2: 2 of 5 subagents time out — the 3-vote "3-of-5" review is
/// `PartialEq`-identical to the same votes framed as "3-of-3": the
/// expected-contributor set is unrepresentable, the two missing
/// contributions (r4, r5, timed out) appear nowhere, and "3 of 5" is
/// indistinguishable from "5 of 5" downstream. No quorum constant
/// exists (`REVIEWERS_MAX` bounds the votes slice; it is not
/// re-exported and is not a quorum of expected contributors).
#[test]
fn partial_returns_are_indistinguishable_from_complete() {
    let report = task_74::run_case("partial_returns_indistinguishable")
        .unwrap_or_else(|e| panic!("task-74 case failed to run: {e}"));
    assert!(
        report.passed,
        "partial case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["votes"], 3);
    assert_eq!(report.metrics["expected"], 5);
    assert_eq!(report.metrics["missing_recorded"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("PartialEq"),
        "evidence must name the identity:\n{joined}"
    );
    assert!(
        joined.contains("cannot distinguish \"3 of 5\" from \"5 of 5\""),
        "evidence must name the downstream indistinguishability:\n{joined}"
    );
}

// --- adversarial ---

/// A1: all subagents fail — zero votes arrive. The design demands a
/// typed `insufficient_contributions` verdict; the real code returns
/// `CouncilError::TooManyItems { field: "reviewers", max: 8 }`, whose
/// Display reads "reviewers holds more than 8 items" for ZERO
/// reviewers — no such variant exists, and the name is misleading for
/// the empty case.
#[test]
fn all_fail_yields_no_typed_insufficient_contributions() {
    let report = task_74::run_case("all_fail_no_typed_verdict")
        .unwrap_or_else(|e| panic!("task-74 case failed to run: {e}"));
    assert!(
        report.passed,
        "all-fail case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["votes"], 0);
    assert_eq!(report.metrics["typed_insufficient_contributions"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("TooManyItems"),
        "evidence must name the actual error variant:\n{joined}"
    );
    assert!(
        joined.contains("more than 8 items"),
        "evidence must show the misleading Display for zero reviewers:\n{joined}"
    );
}

/// A2: a late result arrives AFTER aggregation — rebuilding the review
/// with the late vote silently moves the verdict Keep -> Revise with
/// no late marker, no generation token, no audit trail. The design's
/// "recorded as late, never merged into the published verdict" does
/// not exist: `decision()` is a pure function of the votes slice.
#[test]
fn late_arrival_silently_changes_the_published_verdict() {
    let report = task_74::run_case("late_arrival_silently_merges")
        .unwrap_or_else(|e| panic!("task-74 case failed to run: {e}"));
    assert!(
        report.passed,
        "late-arrival case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["first"], "Keep");
    assert_eq!(report.metrics["second"], "Revise");
    assert_eq!(report.metrics["late_marker"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("Keep -> Revise"),
        "evidence must show the silent verdict move:\n{joined}"
    );
}

/// The task-level driver runs all four cases and reports the honest
/// seam failure: CouncilReview is a votes-only tally that cannot
/// account for gaps — "3 of 5" is indistinguishable from "3 of 3",
/// the empty case yields a misnamed error, and late votes merge
/// silently.
#[test]
fn task_level_verdict_is_fail_at_seam() {
    let ctx = Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-74: cannot build Ctx: {e}"));
    let (where_, how, evidence) = match task_74::run(&ctx) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-74 passed: an aggregation envelope was invented, not found\nevidence: {evidence:?}"
        ),
    };
    assert_eq!(where_, "seam", "task-74 must fail at the seam");
    let joined = evidence.join("\n");
    assert!(
        joined.contains("PartialEq-identical"),
        "evidence must name the core identity:\n{joined}"
    );
    assert!(
        how.contains("product decision for Matt"),
        "the 'how' must bank the product decision: {how}"
    );
    assert!(
        how.contains("votes-only tally"),
        "the 'how' must name the votes-only tally: {how}"
    );
}
