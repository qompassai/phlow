//! Adversarial tests: workflow misuse is rejected with typed errors —
//! never panics, never skips verification, never loses lineage.

use phlow_council::{
    CANDIDATES_MAX, CandidateId, CandidateStatus, CouncilError, CouncilReview, Evidence,
    EvidenceKind, TaskContract, Vote, Workflow,
};

fn contract() -> TaskContract {
    TaskContract::new("goal", &["c1"], &["a1"]).expect("valid contract")
}

fn workflow() -> Workflow {
    Workflow::new(contract())
}

#[test]
fn implement_unknown_candidate_is_rejected() {
    let mut workflow = workflow();
    assert_eq!(
        workflow.implement(CandidateId(99)),
        Err(CouncilError::UnknownCandidate { id: 99 })
    );
}

#[test]
fn child_of_unknown_parent_is_rejected() {
    let mut workflow = workflow();
    assert_eq!(
        workflow.propose_child(CandidateId(7), "orphan"),
        Err(CouncilError::UnknownCandidate { id: 7 })
    );
}

#[test]
fn evidence_for_unknown_candidate_is_rejected() {
    let mut workflow = workflow();
    let evidence = Evidence::new(CandidateId(42), EvidenceKind::Profile, true, "fast")
        .expect("valid evidence");
    assert_eq!(
        workflow.attach_evidence(evidence),
        Err(CouncilError::UnknownCandidate { id: 42 })
    );
}

#[test]
fn double_implement_is_rejected() {
    let mut workflow = workflow();
    let id = workflow.propose("candidate").expect("propose");
    workflow.implement(id).expect("implement");
    assert_eq!(
        workflow.implement(id),
        Err(CouncilError::BadTransition {
            id: id.0,
            from: CandidateStatus::Implemented,
            attempted: "implement",
        })
    );
}

#[test]
fn review_of_rejected_candidate_is_rejected() {
    let mut workflow = workflow();
    let id = workflow.propose("candidate").expect("propose");
    let review = CouncilReview::new(id, &[("r1", Vote::Reject)]).expect("valid review");
    workflow.review(review).expect("reject");
    assert_eq!(workflow.status(id), Ok(CandidateStatus::Rejected));
    let again = CouncilReview::new(id, &[("r1", Vote::Keep)]).expect("valid review");
    assert_eq!(
        workflow.review(again),
        Err(CouncilError::BadTransition {
            id: id.0,
            from: CandidateStatus::Rejected,
            attempted: "review",
        })
    );
}

#[test]
fn review_of_promoted_candidate_is_rejected() {
    let mut workflow = workflow();
    let id = workflow.propose("candidate").expect("propose");
    workflow.implement(id).expect("implement");
    let evidence =
        Evidence::new(id, EvidenceKind::Correctness, true, "passes").expect("valid evidence");
    workflow.attach_evidence(evidence).expect("attach");
    let review = CouncilReview::new(id, &[("r1", Vote::Keep)]).expect("valid review");
    workflow.review(review).expect("promote");
    let again = CouncilReview::new(id, &[("r1", Vote::Revise)]).expect("valid review");
    assert_eq!(
        workflow.review(again),
        Err(CouncilError::BadTransition {
            id: id.0,
            from: CandidateStatus::Promoted,
            attempted: "review",
        })
    );
}

#[test]
fn empty_goal_is_rejected() {
    assert_eq!(
        TaskContract::new("   ", &[], &[]),
        Err(CouncilError::EmptyGoal)
    );
}

#[test]
fn overlong_summary_is_rejected() {
    let mut workflow = workflow();
    let long = "x".repeat(1025);
    assert!(matches!(
        workflow.propose(&long),
        Err(CouncilError::TextTooLong {
            field: "candidate summary",
            max: 1024,
            got: 1025
        })
    ));
}

#[test]
fn too_many_reviewers_are_rejected() {
    let votes: Vec<(&str, Vote)> = vec![("r", Vote::Keep); 9];
    assert_eq!(
        CouncilReview::new(CandidateId(0), &votes),
        Err(CouncilError::TooManyItems {
            field: "reviewers",
            max: 8
        })
    );
}

#[test]
fn duplicate_reviewer_is_rejected() {
    assert_eq!(
        CouncilReview::new(CandidateId(0), &[("r1", Vote::Keep), ("r1", Vote::Reject)]),
        Err(CouncilError::DuplicateReviewer {
            name: "r1".to_string()
        })
    );
}

#[test]
fn candidate_cap_is_enforced() {
    let mut workflow = workflow();
    for _ in 0..CANDIDATES_MAX {
        workflow.propose("candidate").expect("propose");
    }
    assert_eq!(
        workflow.propose("one too many"),
        Err(CouncilError::CandidateCapReached)
    );
}
