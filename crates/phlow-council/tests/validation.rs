//! Validation tests: the workflow moves candidates forward as documented.

use phlow_council::{
    CandidateId, CandidateStatus, CouncilReview, Decision, Evidence, EvidenceKind, TaskContract,
    Vote, Workflow,
};

fn contract() -> TaskContract {
    TaskContract::new(
        "speed up tile matmul",
        &["no unsafe code", "CPU-only"],
        &["tests pass", "profile shows improvement"],
    )
    .expect("valid contract")
}

fn workflow() -> Workflow {
    Workflow::new(contract())
}

fn passing_correctness(candidate: CandidateId) -> Evidence {
    Evidence::new(
        candidate,
        EvidenceKind::Correctness,
        true,
        "24/24 tests pass",
    )
    .expect("valid evidence")
}

#[test]
fn propose_assigns_ids_and_starts_proposed() {
    let mut workflow = workflow();
    let first = workflow.propose("try tiling x").expect("propose");
    let second = workflow.propose("try tiling y").expect("propose");
    assert_eq!(first, CandidateId(0));
    assert_eq!(second, CandidateId(1));
    assert_eq!(workflow.status(first), Ok(CandidateStatus::Proposed));
}

#[test]
fn full_pipeline_propose_implement_verify_promote() {
    let mut workflow = workflow();
    let id = workflow.propose("tile the inner loop").expect("propose");
    workflow.implement(id).expect("implement");
    assert_eq!(workflow.status(id), Ok(CandidateStatus::Implemented));
    workflow
        .attach_evidence(passing_correctness(id))
        .expect("evidence");
    assert_eq!(workflow.status(id), Ok(CandidateStatus::Verified));
    let review =
        CouncilReview::new(id, &[("r1", Vote::Keep), ("r2", Vote::Keep)]).expect("valid review");
    let decision = workflow.review(review).expect("review");
    assert_eq!(decision, Decision::Keep);
    assert_eq!(workflow.status(id), Ok(CandidateStatus::Promoted));
}

#[test]
fn profile_evidence_does_not_change_status() {
    let mut workflow = workflow();
    let id = workflow.propose("candidate").expect("propose");
    workflow.implement(id).expect("implement");
    let profile =
        Evidence::new(id, EvidenceKind::Profile, true, "1.8x faster").expect("valid evidence");
    workflow.attach_evidence(profile).expect("attach");
    assert_eq!(workflow.status(id), Ok(CandidateStatus::Implemented));
    assert_eq!(workflow.evidence_for(id).len(), 1);
}

#[test]
fn failed_correctness_keeps_implemented_and_retains_evidence() {
    let mut workflow = workflow();
    let id = workflow.propose("candidate").expect("propose");
    workflow.implement(id).expect("implement");
    let failed = Evidence::new(id, EvidenceKind::Correctness, false, "3 tests fail")
        .expect("valid evidence");
    workflow.attach_evidence(failed).expect("attach");
    assert_eq!(workflow.status(id), Ok(CandidateStatus::Implemented));
    assert!(!workflow.evidence_for(id)[0].passed);
}

#[test]
fn keep_vote_without_verification_does_not_promote() {
    let mut workflow = workflow();
    let id = workflow.propose("candidate").expect("propose");
    workflow.implement(id).expect("implement");
    let review = CouncilReview::new(id, &[("r1", Vote::Keep)]).expect("valid review");
    let decision = workflow.review(review).expect("review");
    assert_eq!(decision, Decision::Keep);
    // The decision is keep, but only verified candidates promote.
    assert_eq!(workflow.status(id), Ok(CandidateStatus::Implemented));
}

#[test]
fn revise_returns_candidate_to_proposed() {
    let mut workflow = workflow();
    let id = workflow.propose("candidate").expect("propose");
    workflow.implement(id).expect("implement");
    let review = CouncilReview::new(id, &[("r1", Vote::Revise)]).expect("valid review");
    assert_eq!(workflow.review(review), Ok(Decision::Revise));
    assert_eq!(workflow.status(id), Ok(CandidateStatus::Proposed));
    // Rework can proceed: implement again.
    workflow.implement(id).expect("re-implement");
}

#[test]
fn child_proposal_records_parent_lineage() {
    let mut workflow = workflow();
    let parent = workflow.propose("v1").expect("propose");
    let child = workflow
        .propose_child(parent, "v2 revises v1")
        .expect("child");
    assert_eq!(workflow.parent_of(child), Ok(Some(parent)));
    assert_eq!(workflow.parent_of(parent), Ok(None));
}

#[test]
fn majority_keep_is_keep() {
    let review = CouncilReview::new(
        CandidateId(0),
        &[("a", Vote::Keep), ("b", Vote::Keep), ("c", Vote::Revise)],
    )
    .expect("valid review");
    assert_eq!(review.decision(), Decision::Keep);
}

#[test]
fn tie_votes_yield_revise() {
    let review = CouncilReview::new(CandidateId(0), &[("a", Vote::Keep), ("b", Vote::Reject)])
        .expect("valid review");
    assert_eq!(review.decision(), Decision::Revise);
}

#[test]
fn correctness_evidence_on_proposed_does_not_verify() {
    // Baseline-style evidence may attach early, but only an implemented
    // candidate with passing correctness becomes verified.
    let mut workflow = workflow();
    let id = workflow.propose("candidate").expect("propose");
    let evidence =
        Evidence::new(id, EvidenceKind::Correctness, true, "passes").expect("valid evidence");
    workflow.attach_evidence(evidence).expect("attach");
    assert_eq!(workflow.status(id), Ok(CandidateStatus::Proposed));
}

#[test]
fn unanimous_reject_is_reject() {
    let review = CouncilReview::new(CandidateId(0), &[("a", Vote::Reject), ("b", Vote::Reject)])
        .expect("valid review");
    assert_eq!(review.decision(), Decision::Reject);
}
