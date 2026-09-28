//! Integration tests for task-19 (council arbitration).
//!
//! 50/50 split against phlow's real council tally
//! (`phlow_council::CouncilReview::decision`):
//!
//! - V1: strict pluralities win — keep, revise, and reject majorities,
//!   plus the unanimous single-reviewer edge.
//! - V2: every tie resolves to `Revise`, the documented safe default,
//!   with an operator-view line that says so.
//! - A1: exhaustive proof over all 120 vote distributions for councils of
//!   1-4 reviewers: a tie can never yield `Keep`.
//! - A2: adversarial coalitions cannot manufacture `Keep` without a strict
//!   plurality; duplicate reviewers, empty councils, and oversized
//!   councils are rejected by the constructor.

use phlow_council::{Decision, Vote};
use phlow_gauntlet::tasks::task_19;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` for one test. This task drives no Neovim and no nvim-lua
/// driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("unused: task-19 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-19 is TaskKind::Rust, no diver lua involved"),
        std::env::temp_dir().join("gauntlet-task-19"),
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_19::CaseReport {
    let report = task_19::run_case(case);
    assert_eq!(report.case, case, "verdict case mismatch");
    report
}

/// Run the full driver; unwrap the Pass outcome or fail with the driver's
/// own evidence attached.
fn run_pass() -> Vec<String> {
    match task_19::run(&test_ctx()) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-19 driver failed at {where_}: {how}\nevidence: {evidence:?}"),
    }
}

// --- validation ---

/// V: a strict plurality wins for each decision kind; a unanimous single
/// reviewer decides Keep. Also pins the task metadata contract.
#[test]
fn strict_plurality_wins() {
    assert_eq!(task_19::ID, "task-19");
    assert_eq!(task_19::NAME, "council arbitration");
    assert_eq!(task_19::KIND, TaskKind::Rust);
    assert_eq!(
        task_19::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("majority-wins");
    assert!(report.passed, "majority-wins failed: {:?}", report.failures);
    let joined = report.evidence.join("\n");
    for decision in ["decision=Keep", "decision=Revise", "decision=Reject"] {
        assert!(
            joined.contains(decision),
            "expected an operator view with {decision}, got:\n{joined}"
        );
    }
    // The full driver aggregates all four cases into one Pass.
    let evidence = run_pass();
    let joined = evidence.join("\n");
    assert!(
        joined.contains("case majority-wins: passed=true"),
        "driver did not pass majority-wins:\n{joined}"
    );
}

/// V: every tie resolves to Revise — keep/revise, keep/reject, three-way,
/// and a 6-reviewer tie — and the operator view names the safe default.
#[test]
fn every_tie_resolves_to_revise() {
    let report = run_case("tie-revises");
    assert!(report.passed, "tie-revises failed: {:?}", report.failures);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("tie → safe default Revise"),
        "expected the safe-default note in the operator view, got:\n{joined}"
    );
    assert!(
        !joined.contains("decision=Keep"),
        "a tie decided Keep — the safe default is broken:\n{joined}"
    );
    assert!(
        joined.contains("tie correctly resolved to the safe default Revise"),
        "expected explicit tie resolutions, got:\n{joined}"
    );
}

// --- adversarial ---

/// A: exhaustive — all 120 vote distributions for councils of 1-4
/// reviewers; `Keep` occurs if and only if keep holds a strict plurality.
/// No tie, of any shape, may yield `Keep`.
#[test]
fn no_tie_can_ever_yield_keep() {
    let report = run_case("exhaustive-no-keep-on-tie");
    assert!(
        report.passed,
        "exhaustive-no-keep-on-tie failed: {:?}",
        report.failures
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("exhaustively checked 120 vote distributions"),
        "expected the 120-distribution proof, got:\n{joined}"
    );
    assert!(
        joined.contains("zero ties yielded Keep"),
        "expected the zero-ties verdict, got:\n{joined}"
    );
    if let Some(failure) = report.failures.first() {
        panic!("exhaustive proof found a violation: {failure}");
    }
}

/// A: coalitions forcing keep/reject and keep/revise ties get Revise, not
/// Keep; the constructor rejects ballot-stuffing (duplicate reviewer),
/// empty councils, and oversized councils.
#[test]
fn tie_forcing_coalitions_and_ballot_stuffing_repelled() {
    let report = run_case("tie-forcing-attack");
    assert!(
        report.passed,
        "tie-forcing-attack failed: {:?}",
        report.failures
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("attack 1 repelled: keep/reject 2-2 tie resolves Revise, not Keep"),
        "expected attack 1 repelled, got:\n{joined}"
    );
    assert!(
        joined.contains("attack 2 repelled: keep/revise 3-3 tie resolves Revise, not Keep"),
        "expected attack 2 repelled, got:\n{joined}"
    );
    assert!(
        joined.contains("ballot-stuffing rejected: duplicate reviewer 'mallory'"),
        "expected the duplicate reviewer rejected, got:\n{joined}"
    );
    assert!(
        joined.contains("oversized council (9 > REVIEWERS_MAX=8) rejected"),
        "expected the oversized council rejected, got:\n{joined}"
    );
    // The honest boundary still works: keep 3 vs 2/2 is a real plurality.
    assert!(
        joined.contains("keep 3 vs 2/2 is a strict plurality, correctly Keep"),
        "expected the honest near-miss boundary, got:\n{joined}"
    );
    // Sanity on the imported types: the test really drove the council API.
    assert_ne!(Decision::Keep, Decision::Revise);
    assert_ne!(Vote::Keep, Vote::Reject);
}
