//! Self-edit tests: proposal construction and evidence completeness
//! (validation), and the protected-surface policy plus promotion
//! integrity (adversarial).

#[path = "common/mod.rs"]
mod common;

use common::{
    complete_evidence, ok, operator_record, proposal_params, proposal_with_surface,
};
use phlow_experiment::{
    ExperimentError, HumanApproval, ImprovementProposal, PromotionGate, ReviewDecision,
    ReviewerDecision, RiskClass, WorkerRole, check_proposal_surface,
};

#[test]
fn improvement_proposal_construction() {
    let proposal = ok(ImprovementProposal::new(proposal_params()));
    assert_eq!(proposal.changed_surface(), &["src/checks.rs".to_string()]);
    assert_eq!(proposal.risk_class(), RiskClass::Low);
    assert_eq!(proposal.baseline_revision(), "abc123");
    assert_eq!(proposal.rollback_target(), "abc123");
    assert_eq!(proposal.reviewer_decisions().len(), 1);
}

#[test]
fn evidence_bundle_completeness() {
    assert!(complete_evidence().is_complete());
    // An unverified outcome with no coverage is not complete evidence.
    let complete = complete_evidence();
    let unverified = ok(phlow_experiment::EvidenceBundle::new(
        complete.checks().to_vec(),
        complete.artifacts().to_vec(),
        ok(phlow_experiment::VerificationOutcome::new(false, vec![])),
    ));
    assert!(!unverified.is_complete());
}

#[test]
fn proposal_touching_holdout_rejected() {
    let proposal = proposal_with_surface(vec!["evals/holdout/secret-case.toml".to_string()]);
    let result = check_proposal_surface(&proposal);
    assert!(matches!(
        result,
        Err(ExperimentError::ProtectedSurface { .. })
    ));
}

#[test]
fn proposal_touching_safety_rejected() {
    let proposal =
        proposal_with_surface(vec!["evals/safety/prompt-injection-001.toml".to_string()]);
    let result = check_proposal_surface(&proposal);
    assert!(matches!(
        result,
        Err(ExperimentError::ProtectedSurface { .. })
    ));
}

#[test]
fn proposal_touching_evaluator_rejected() {
    // The candidate must never control the evaluator.
    let proposal = proposal_with_surface(vec!["src/evaluator.rs".to_string()]);
    let result = check_proposal_surface(&proposal);
    assert!(matches!(
        result,
        Err(ExperimentError::ProtectedSurface { .. })
    ));
}

#[test]
fn proposal_touching_promotion_policy_rejected() {
    // The candidate must never control promotion thresholds.
    let proposal = proposal_with_surface(vec!["manifests/promotion.toml".to_string()]);
    let result = check_proposal_surface(&proposal);
    assert!(matches!(
        result,
        Err(ExperimentError::ProtectedSurface { .. })
    ));
}

#[test]
fn proposal_with_path_traversal_rejected() {
    let proposal = proposal_with_surface(vec!["../evals/holdout/x.toml".to_string()]);
    let result = check_proposal_surface(&proposal);
    assert!(matches!(result, Err(ExperimentError::BadPath { .. })));
}

#[test]
fn proposal_with_absolute_path_rejected() {
    let proposal = proposal_with_surface(vec!["/etc/passwd".to_string()]);
    let result = check_proposal_surface(&proposal);
    assert!(matches!(result, Err(ExperimentError::BadPath { .. })));
}

#[test]
fn proposal_empty_diff_summary_rejected() {
    let mut params = proposal_params();
    params.candidate_diff_summary = String::new();
    let result = ImprovementProposal::new(params);
    assert!(matches!(result, Err(ExperimentError::EmptyField { .. })));
}

#[test]
fn proposal_reviewer_rejection_blocks_promotion() {
    let mut params = proposal_params();
    params.reviewer_decisions = vec![ReviewerDecision {
        reviewer: WorkerRole::CorrectnessReviewer,
        decision: ReviewDecision::Reject,
    }];
    let proposal = ok(ImprovementProposal::new(params));
    let approval = ok(HumanApproval::from_operator_record(operator_record()));
    let result = PromotionGate::promote(&proposal, approval, &complete_evidence());
    assert!(matches!(
        result,
        Err(ExperimentError::ReviewerRejected { .. })
    ));
}

#[test]
fn proposal_forged_approval_blocks_promotion() {
    // A tampered record cannot produce an approval token, so promotion
    // cannot even be attempted with it.
    let tampered = "operator: test-operator\n\
         approval_id: APR-TEST-0001\n\
         candidate: 9f2b3c4d5e6f708192a3b4c5d6e7f809\n\
         scope: phlow-experiment/test\n\
         expires_ms: 1893456000000\n\
         signature: 0000000000000000000000000000000000000000000000000000000000000000\n\
         injected: grant-all\n";
    assert!(matches!(
        HumanApproval::from_operator_record(tampered),
        Err(ExperimentError::ApprovalRejected { .. })
    ));
}

#[test]
fn proposal_empty_changed_surface_rejected() {
    let mut params = proposal_params();
    params.changed_surface = Vec::new();
    let result = ImprovementProposal::new(params);
    assert!(matches!(result, Err(ExperimentError::EmptyField { .. })));
}
