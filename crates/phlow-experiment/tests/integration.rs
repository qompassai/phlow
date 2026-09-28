//! Integration tests: the skeleton flow end to end, the manifest → types →
//! record round trip, a fully-gated promotion, scheduler admission, and
//! evaluator stage order.

#[path = "common/mod.rs"]
mod common;

use common::{
    complete_evidence, eval_text, ok, operator_record, proposal_params, test_budget, test_node,
    test_scheduler,
};
use phlow_experiment::{
    CheckRecord, EvaluationRecord, Evaluator, ExperimentError, HumanApproval, ImprovementProposal,
    Lifecycle, LifecycleEvent, NodeId, NodeState, PromotionGate, RecordParams, parse_task_manifest,
};

#[test]
fn full_lifecycle_to_awaiting_human_then_denied() {
    let to_awaiting = || {
        let state = ok(Lifecycle::Proposed.transition(LifecycleEvent::WorkspaceCreated));
        let state = ok(state.transition(LifecycleEvent::ChecksComplete));
        let state = ok(state.transition(LifecycleEvent::EvaluationComplete));
        ok(state.transition(LifecycleEvent::GatesSatisfied))
    };
    assert_eq!(to_awaiting(), Lifecycle::AwaitingHuman);
    // Denied approval rejects.
    let rejected = ok(to_awaiting().transition(LifecycleEvent::ApprovalDenied));
    assert_eq!(rejected, Lifecycle::Rejected);
    // Expired approval rejects too.
    let rejected = ok(to_awaiting().transition(LifecycleEvent::ApprovalExpired));
    assert_eq!(rejected, Lifecycle::Rejected);
}

#[test]
fn manifest_to_record_round_trip() {
    let manifest = ok(parse_task_manifest(
        &eval_text("evals/public/rust-cli-parse-001.toml"),
        "evals/public/rust-cli-parse-001.toml",
    ));
    assert_eq!(manifest.id, "rust-cli-parse-001");
    assert_eq!(manifest.checks.len(), 3);
    assert!(manifest.checks.iter().all(|c| c.required));

    let mut record = ok(EvaluationRecord::new(RecordParams {
        experiment_id: manifest.id.clone(),
        baseline_revision: "abc123".to_string(),
        workspace_digest: "workspace-digest-001".to_string(),
        operator_config_digest: "operator-config-digest-001".to_string(),
        model_ids: vec!["test-model-1".to_string()],
        toolchain_versions: vec!["rustc 1.90.0".to_string()],
        limits: vec!["budgets.toml defaults".to_string()],
        stop_reason: "trial complete".to_string(),
    }));
    // Constructor defaults: nothing verified, nothing eligible, nothing approved.
    assert!(!record.verified());
    assert!(!record.promotion_eligible());
    assert!(!record.human_approved());
    assert_eq!(record.schema_version(), 1);

    ok(record.record_event("task admitted"));
    for check in &manifest.checks {
        ok(record.record_check(ok(CheckRecord::new(
            &check.name,
            check.argv.clone(),
            true,
            check.required,
        ))));
    }
    assert_eq!(record.check_count(), 3);

    let json = ok(record.to_json());
    for expected in [
        "\"schema_version\": 1",
        "\"experiment_id\": \"rust-cli-parse-001\"",
        "\"status\": \"experimental\"",
        "\"verified\": false",
        "\"eligible\": false",
        "\"human_approved\": false",
        "\"stop_reason\": \"trial complete\"",
    ] {
        assert!(json.contains(expected), "record JSON missing {expected}");
    }
}

#[test]
fn promotion_with_valid_approval_and_evidence() {
    let proposal = ok(ImprovementProposal::new(proposal_params()));
    let approval = ok(HumanApproval::from_operator_record(operator_record()));
    let evidence = complete_evidence();
    let record = ok(PromotionGate::promote(&proposal, approval, &evidence));
    assert_eq!(record.operator(), "test-operator");
    assert_eq!(record.approval_id(), "APR-TEST-0001");
    assert_eq!(record.rollback_target(), "abc123");
    assert_eq!(record.evidence_checks(), 1);
}

#[test]
fn scheduler_admit_publish_flow() {
    let mut scheduler = test_scheduler();
    ok(scheduler.admit(test_node("n1")));
    assert_eq!(scheduler.queue_len(), 1);
    let node_id = ok(NodeId::new("n1"));
    ok(scheduler.publish_result(&node_id, 0, "result-digest-1", NodeState::Succeeded));
    assert_eq!(scheduler.queue_len(), 0);
    assert_eq!(scheduler.published_count(), 1);
    let node = ok(scheduler
        .node(&node_id)
        .ok_or(ExperimentError::UnknownNode {
            id: "n1".to_string(),
        }));
    assert_eq!(node.state(), NodeState::Succeeded);
    assert_eq!(node.result_digest(), Some("result-digest-1"));
}

#[test]
fn evaluator_stage_order() {
    let mut evaluator = Evaluator::new(test_budget());
    // Out-of-order stages are rejected.
    let early = evaluator.prepare();
    assert!(matches!(early, Err(ExperimentError::BadStageOrder { .. })));
    // In order: validate -> prepare -> execute -> verify -> review -> promote.
    ok(evaluator.validate());
    ok(evaluator.prepare());
    ok(evaluator.execute(1, 16));
    let verified = ok(evaluator.verify(&complete_evidence()));
    assert!(verified);
    ok(evaluator.review());
    ok(evaluator.promote());
    // Promote is terminal: a second call is rejected.
    let again = evaluator.promote();
    assert!(matches!(again, Err(ExperimentError::BadStageOrder { .. })));
}
