//! task-19: council arbitration (rust).
//!
//! Drives phlow's real council tally — [`phlow_council::CouncilReview::decision`]
//! (`crates/phlow-council/src/review.rs`) — through conflicting agent
//! outputs. N reviewers vote keep/revise/reject; the driver proves:
//!
//! - a strict plurality wins (keep, revise, or reject),
//! - every tie resolves to `Revise` — the code's documented safe default
//!   ("more work, not a verdict"),
//! - by exhaustive enumeration over all small-council vote distributions,
//!   a tie can **never** yield `Keep` (keep wins only on strict plurality
//!   over both other options),
//! - adversarial coalitions cannot manufacture `Keep` without a strict
//!   plurality, and ballot-stuffing (duplicate reviewers) is rejected by
//!   the constructor.
//!
//! Design note the driver surfaces rather than hides: a reject/revise tie
//! resolves to `Reject` (`reject > keep && reject >= revise`), i.e. the
//! terminal verdict wins ties against rework. That is the documented rule;
//! the doc records it as an explicit design observation.

use crate::{Ctx, TaskKind, TaskOutcome, bound_evidence};
use phlow_council::{CandidateId, CouncilError, CouncilReview, Decision, Vote};

/// Task id.
pub const ID: &str = "task-19";
/// Human-readable name.
pub const NAME: &str = "council arbitration";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// The arbitration cases the driver runs, in order:
/// two validation, two adversarial.
pub const CASES: [&str; 4] = [
    "majority-wins",
    "tie-revises",
    "exhaustive-no-keep-on-tie",
    "tie-forcing-attack",
];

/// One row of the validation matrix: scenario label, ballots, expected decision.
type Case<'a> = (&'a str, &'a [(&'a str, Vote)], Decision);

/// The observed result of one arbitration case.
#[derive(Debug, Clone)]
pub struct CaseReport {
    /// Which case ran.
    pub case: &'static str,
    /// True when every assertion in the case held.
    pub passed: bool,
    /// Diagnostic lines from the case.
    pub evidence: Vec<String>,
    /// Failing assertion details, empty when `passed`.
    pub failures: Vec<String>,
}

/// Build a review for `votes` on a throwaway candidate. The candidate id
/// is irrelevant to the tally; only the votes matter.
fn review(votes: &[(&str, Vote)]) -> CouncilReview {
    CouncilReview::new(CandidateId(0), votes).expect("task-19 fixture: valid votes")
}

/// Count (keep, revise, reject) in a vote slice.
fn tally(votes: &[(&str, Vote)]) -> (usize, usize, usize) {
    let (mut keep, mut revise, mut reject) = (0, 0, 0);
    for (_, vote) in votes {
        match vote {
            Vote::Keep => keep += 1,
            Vote::Revise => revise += 1,
            Vote::Reject => reject += 1,
        }
    }
    (keep, revise, reject)
}

/// The operator-facing verdict line: what a human reads after the council
/// votes. One line, unambiguous decision, full tally, and — on ties — the
/// safe-default note.
fn operator_view(votes: &[(&str, Vote)], decision: Decision) -> String {
    let (keep, revise, reject) = tally(votes);
    // The safe default fired exactly when the decision is Revise without a
    // revise plurality — i.e. the fallthrough, not a win.
    let safe_default = decision == Decision::Revise && !(revise > keep && revise > reject);
    let note = if safe_default {
        " — tie → safe default Revise"
    } else {
        ""
    };
    format!(
        "operator view: decision={decision:?} (keep={keep} revise={revise} reject={reject}){note}"
    )
}

/// Check one vote distribution against its expected decision, recording
/// evidence. Returns the failure detail on mismatch.
fn expect_decision(
    evidence: &mut Vec<String>,
    label: &str,
    votes: &[(&str, Vote)],
    expected: Decision,
) -> Option<String> {
    let decided = review(votes).decision();
    evidence.push(operator_view(votes, decided));
    if decided != expected {
        Some(format!(
            "{label}: decided {decided:?}, expected {expected:?} for votes {votes:?}"
        ))
    } else {
        None
    }
}

/// V1: a strict plurality wins — keep, revise, and reject majorities each
/// produce their own decision, plus the unanimous single-reviewer edge.
fn case_majority_wins() -> CaseReport {
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    let ballots: &[Case] = &[
        (
            "keep-majority",
            &[
                ("a", Vote::Keep),
                ("b", Vote::Keep),
                ("c", Vote::Keep),
                ("d", Vote::Revise),
                ("e", Vote::Reject),
            ],
            Decision::Keep,
        ),
        (
            "revise-majority",
            &[
                ("a", Vote::Revise),
                ("b", Vote::Revise),
                ("c", Vote::Revise),
                ("d", Vote::Keep),
                ("e", Vote::Reject),
            ],
            Decision::Revise,
        ),
        (
            "reject-majority",
            &[
                ("a", Vote::Reject),
                ("b", Vote::Reject),
                ("c", Vote::Reject),
                ("d", Vote::Keep),
                ("e", Vote::Revise),
            ],
            Decision::Reject,
        ),
        ("unanimous-single", &[("solo", Vote::Keep)], Decision::Keep),
    ];
    for (label, votes, expected) in ballots {
        if let Some(failure) = expect_decision(&mut evidence, label, votes, *expected) {
            failures.push(failure);
        }
    }
    CaseReport {
        case: "majority-wins",
        passed: failures.is_empty(),
        evidence,
        failures,
    }
}

/// V2: every tie resolves to `Revise` — two-way ties, the three-way tie,
/// and the keep/reject tie that must never yield `Keep`.
fn case_tie_revises() -> CaseReport {
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    let ballots: &[(&str, &[(&str, Vote)])] = &[
        (
            "keep-revise-tie",
            &[
                ("a", Vote::Keep),
                ("b", Vote::Keep),
                ("c", Vote::Revise),
                ("d", Vote::Revise),
            ],
        ),
        (
            "three-way-tie",
            &[("a", Vote::Keep), ("b", Vote::Revise), ("c", Vote::Reject)],
        ),
        (
            "keep-reject-tie",
            &[
                ("a", Vote::Keep),
                ("b", Vote::Keep),
                ("c", Vote::Reject),
                ("d", Vote::Reject),
            ],
        ),
        (
            "large-keep-revise-tie",
            &[
                ("a", Vote::Keep),
                ("b", Vote::Keep),
                ("c", Vote::Keep),
                ("d", Vote::Revise),
                ("e", Vote::Revise),
                ("f", Vote::Revise),
            ],
        ),
    ];
    for (label, votes) in ballots {
        if let Some(failure) = expect_decision(&mut evidence, label, votes, Decision::Revise) {
            failures.push(failure);
        } else {
            evidence.push(format!(
                "{label}: tie correctly resolved to the safe default Revise"
            ));
        }
    }
    CaseReport {
        case: "tie-revises",
        passed: failures.is_empty(),
        evidence,
        failures,
    }
}

/// A1: exhaustive proof that a tie can never yield `Keep`. Enumerates all
/// 3^n vote distributions for councils of 1..=4 reviewers (120 total) and
/// asserts the exact contract against the real tally: `Keep` if and only
/// if keep holds a strict plurality over both revise and reject. Any
/// distribution where keep merely ties — with either or both opponents —
/// must not decide `Keep`.
fn case_exhaustive_no_keep_on_tie() -> CaseReport {
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    let mut checked: u64 = 0;
    let mut keep_wins: u64 = 0;
    // 3^n distributions, reviewer names r0..rn-1.
    for n in 1..=4usize {
        let total = 3usize.pow(n as u32);
        for combo in 0..total {
            let names: Vec<String> = (0..n).map(|i| format!("r{i}")).collect();
            let mut votes: Vec<(&str, Vote)> = Vec::with_capacity(n);
            let mut digits = combo;
            for name in &names {
                let vote = match digits % 3 {
                    0 => Vote::Keep,
                    1 => Vote::Revise,
                    _ => Vote::Reject,
                };
                votes.push((name.as_str(), vote));
                digits /= 3;
            }
            let (keep, revise, reject) = tally(&votes);
            let strict_plurality = keep > revise && keep > reject;
            let decided = review(&votes).decision();
            checked += 1;
            if strict_plurality {
                keep_wins += 1;
                if decided != Decision::Keep {
                    failures.push(format!(
                        "strict keep plurality ({keep}/{revise}/{reject}) did not decide Keep: {decided:?}"
                    ));
                }
            } else if decided == Decision::Keep {
                failures.push(format!(
                    "TIE YIELDED KEEP: ({keep}/{revise}/{reject}) decided Keep without strict plurality"
                ));
            }
            // Determinism: the same ballots decide the same way twice.
            if review(&votes).decision() != decided {
                failures.push(format!(
                    "non-deterministic tally for ({keep}/{revise}/{reject})"
                ));
            }
        }
    }
    evidence.push(format!(
        "exhaustively checked {checked} vote distributions (councils of 1-4); \
         strict keep pluralities: {keep_wins}, all decided Keep; \
         zero ties yielded Keep"
    ));
    CaseReport {
        case: "exhaustive-no-keep-on-tie",
        passed: failures.is_empty(),
        evidence,
        failures,
    }
}

/// A2: adversarial coalitions try to manufacture `Keep` without a strict
/// plurality, and a ballot-stuffer tries to vote twice. The tally must
/// hold; the constructor must reject the stuffer.
fn case_tie_forcing_attack() -> CaseReport {
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    // Coalition 1: keep ties reject 2-2. Must NOT be Keep.
    if let Some(failure) = expect_decision(
        &mut evidence,
        "keep-reject-2v2",
        &[
            ("a", Vote::Keep),
            ("b", Vote::Keep),
            ("c", Vote::Reject),
            ("d", Vote::Reject),
        ],
        Decision::Revise,
    ) {
        failures.push(failure);
    } else {
        evidence
            .push("attack 1 repelled: keep/reject 2-2 tie resolves Revise, not Keep".to_string());
    }
    // Coalition 2: keep ties revise 3-3 with a lone reject. Must NOT be Keep.
    if let Some(failure) = expect_decision(
        &mut evidence,
        "keep-revise-3v3",
        &[
            ("a", Vote::Keep),
            ("b", Vote::Keep),
            ("c", Vote::Keep),
            ("d", Vote::Revise),
            ("e", Vote::Revise),
            ("f", Vote::Revise),
            ("g", Vote::Reject),
        ],
        Decision::Revise,
    ) {
        failures.push(failure);
    } else {
        evidence
            .push("attack 2 repelled: keep/revise 3-3 tie resolves Revise, not Keep".to_string());
    }
    // Coalition 3: near-miss — keep 3, revise 2, reject 2 IS a strict
    // plurality (3 > 2 and 3 > 2): honestly Keep. The attacker needs 4.
    if let Some(failure) = expect_decision(
        &mut evidence,
        "keep-near-miss-3v2v2",
        &[
            ("a", Vote::Keep),
            ("b", Vote::Keep),
            ("c", Vote::Keep),
            ("d", Vote::Revise),
            ("e", Vote::Revise),
            ("f", Vote::Reject),
            ("g", Vote::Reject),
        ],
        Decision::Keep,
    ) {
        failures.push(failure);
    } else {
        evidence.push(
            "boundary honest: keep 3 vs 2/2 is a strict plurality, correctly Keep (attacker needs 4)"
                .to_string(),
        );
    }
    // Documented rule under adversarial light: reject/revise 2-2 resolves
    // to Reject — the terminal verdict wins ties against rework. Assert the
    // rule holds as documented (design observation, not a challenge).
    if let Some(failure) = expect_decision(
        &mut evidence,
        "reject-revise-2v2",
        &[
            ("a", Vote::Reject),
            ("b", Vote::Reject),
            ("c", Vote::Revise),
            ("d", Vote::Revise),
        ],
        Decision::Reject,
    ) {
        failures.push(failure);
    } else {
        evidence.push(
            "documented rule holds: reject/revise 2-2 resolves Reject (reject wins ties vs revise)"
                .to_string(),
        );
    }
    // Ballot-stuffing: the same reviewer voting twice is rejected.
    match CouncilReview::new(
        CandidateId(0),
        &[("mallory", Vote::Keep), ("mallory", Vote::Keep)],
    ) {
        Err(CouncilError::DuplicateReviewer { name }) => {
            evidence.push(format!(
                "ballot-stuffing rejected: duplicate reviewer '{name}'"
            ));
        }
        other => failures.push(format!(
            "duplicate reviewer was not rejected with DuplicateReviewer: {other:?}"
        )),
    }
    // Empty and oversized councils are rejected, not tallied.
    if CouncilReview::new(CandidateId(0), &[]).is_ok() {
        failures.push("empty council was accepted".to_string());
    } else {
        evidence.push("empty council rejected".to_string());
    }
    let big_names: Vec<String> = (0..9).map(|i| format!("r{i}")).collect();
    let big_votes: Vec<(&str, Vote)> = big_names
        .iter()
        .map(|name| (name.as_str(), Vote::Revise))
        .collect();
    if CouncilReview::new(CandidateId(0), &big_votes).is_ok() {
        failures.push("oversized council (9 reviewers) was accepted".to_string());
    } else {
        evidence.push("oversized council (9 > REVIEWERS_MAX=8) rejected".to_string());
    }
    CaseReport {
        case: "tie-forcing-attack",
        passed: failures.is_empty(),
        evidence,
        failures,
    }
}

/// Run one arbitration case by name. Unknown names produce a failing
/// report, never a silent pass.
pub fn run_case(case: &str) -> CaseReport {
    match case {
        "majority-wins" => case_majority_wins(),
        "tie-revises" => case_tie_revises(),
        "exhaustive-no-keep-on-tie" => case_exhaustive_no_keep_on_tie(),
        "tie-forcing-attack" => case_tie_forcing_attack(),
        _ => CaseReport {
            case: "unknown",
            passed: false,
            evidence: Vec::new(),
            failures: vec![format!("unknown case '{case}'")],
        },
    }
}

/// Attempt the task: run all four arbitration cases and aggregate.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    let mut evidence = Vec::new();
    let mut fail_where = String::new();
    let mut fail_how = String::new();
    for case in [
        case_majority_wins(),
        case_tie_revises(),
        case_exhaustive_no_keep_on_tie(),
        case_tie_forcing_attack(),
    ] {
        evidence.push(format!("case {}: passed={}", case.case, case.passed));
        for line in &case.evidence {
            evidence.push(format!("case {}: {line}", case.case));
        }
        if !case.passed && fail_where.is_empty() {
            fail_where = case.case.to_string();
            fail_how = case.failures.join("; ");
            if fail_how.is_empty() {
                fail_how = "case reported passed=false with no detail".to_string();
            }
        }
    }
    if fail_where.is_empty() {
        TaskOutcome::Pass {
            evidence: bound_evidence(evidence),
        }
    } else {
        TaskOutcome::Fail {
            where_: fail_where,
            how: fail_how,
            evidence: bound_evidence(evidence),
        }
    }
}
