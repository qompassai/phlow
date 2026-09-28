//! Shared test helpers: explicit inputs, no hidden state.
//!
//! Every helper builds its value from literal arguments so each test states
//! exactly what it feeds the code under test.
//!
//! This module is compiled separately into each integration-test binary and
//! each binary uses only a subset, so per-binary dead-code analysis would
//! flag the rest. The allow below is scoped to that artifact: every helper
//! here is used by at least one test binary (verified by grep 2026-09-28),
//! so it silences subset noise, not genuine disuse.
#![allow(dead_code)]

use phlow_experiment::{
    ArtifactDigest, BudgetTracker, CapabilitySet, CheckRun, EvidenceBundle, ExperimentError,
    ExperimentId, NodeId, NodeParams, ProposalBudgets, ProposalParams, ReviewDecision,
    ReviewerDecision, RiskClass, RunId, Scheduler, SchedulerLimits, SchedulerNode,
    VerificationOutcome, WorkerRole,
};

/// Unwraps a `Result`, panicking with the typed error on failure.
///
/// Tests use this instead of `unwrap()` so a failure prints the
/// domain error, not a bare "called unwrap on Err".
pub fn ok<T>(result: Result<T, ExperimentError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("expected Ok, got error: {error}"),
    }
}

/// Reads a shipped manifest from `manifests/` by file name.
pub fn manifest_text(name: &str) -> String {
    let path = format!("{}/manifests/{name}", env!("CARGO_MANIFEST_DIR"));
    match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => panic!("cannot read manifest {path}: {error}"),
    }
}

/// Reads a shipped eval file by crate-relative path.
pub fn eval_text(relative: &str) -> String {
    let path = format!("{}/{relative}", env!("CARGO_MANIFEST_DIR"));
    match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => panic!("cannot read eval file {path}: {error}"),
    }
}

/// A well-formed operator approval record for tests.
///
/// Shape: the exact six keys, a 32-hex-char candidate digest, a positive
/// `expires_ms`, and a 64-hex-char signature.
pub fn operator_record() -> &'static str {
    "operator: test-operator\n\
     approval_id: APR-TEST-0001\n\
     candidate: 9f2b3c4d5e6f708192a3b4c5d6e7f809\n\
     scope: phlow-experiment/test\n\
     expires_ms: 1893456000000\n\
     signature: abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\n"
}

/// A root capability set for tests: two tools, two paths, positive budgets.
pub fn root_capabilities() -> CapabilitySet {
    ok(CapabilitySet::new(
        vec!["read".to_string(), "exec-check".to_string()],
        vec!["workspace/".to_string(), "fixtures/".to_string()],
        64,
        1_048_576,
    ))
}

/// Constructor params for a test node with id `node`.
pub fn node_params(node: &str) -> NodeParams {
    NodeParams {
        run_id: ok(RunId::new("run-test-001")),
        experiment_id: ok(ExperimentId::new("exp-test-001")),
        baseline_revision: "abc123".to_string(),
        workspace_snapshot: "snap-001".to_string(),
        node_id: ok(NodeId::new(node)),
        parent_node_id: None,
        role: WorkerRole::Implementer,
        capabilities: root_capabilities(),
        input_digest: "input-digest-001".to_string(),
        dependency_ids: Vec::new(),
        generation: 0,
        attempt: 0,
        deadline_ms: 300_000,
        cpu_budget_ms: 60_000,
        memory_budget_bytes: 536_870_912,
        output_bytes_max: 1_048_576,
        tool_calls_remaining: 64,
    }
}

/// A valid test node with id `node`, in `Proposed` state.
pub fn test_node(node: &str) -> SchedulerNode {
    ok(SchedulerNode::new(node_params(node)))
}

/// A scheduler with default limits.
pub fn test_scheduler() -> Scheduler {
    ok(Scheduler::new(SchedulerLimits::default()))
}

/// A complete evidence bundle: one passing check with exact argv, one
/// artifact digest, and a verified outcome with coverage.
pub fn complete_evidence() -> EvidenceBundle {
    let checks = vec![ok(CheckRun::new(
        "tests",
        vec![
            "cargo".to_string(),
            "test".to_string(),
            "--locked".to_string(),
        ],
        true,
    ))];
    let artifacts = vec![ok(ArtifactDigest::new("candidate.diff", "9f2b3c4d"))];
    let verification = ok(VerificationOutcome::new(
        true,
        vec!["src/lib.rs".to_string()],
    ));
    ok(EvidenceBundle::new(checks, artifacts, verification))
}

/// A budget tracker with headroom: 64 tool calls, 1 MiB output, 5 min.
pub fn test_budget() -> BudgetTracker {
    ok(BudgetTracker::new(64, 1_048_576, 300_000))
}

/// Valid proposal params: one changed file, one approving reviewer.
pub fn proposal_params() -> ProposalParams {
    ProposalParams {
        trigger: "REG-2026-001".to_string(),
        failure_category: "timeout-handling".to_string(),
        baseline_revision: "abc123".to_string(),
        candidate_diff_summary: "Tighten the check timeout path.".to_string(),
        changed_surface: vec!["src/checks.rs".to_string()],
        expected_benefit: "Fewer hung checks.".to_string(),
        risk_class: RiskClass::Normal,
        budgets: ProposalBudgets {
            tool_calls_max: 64,
            wall_ms_max: 300_000,
        },
        test_version: "suites-1".to_string(),
        evaluator_version: "evaluator-1".to_string(),
        results_summary: "All gates green.".to_string(),
        reviewer_decisions: vec![ReviewerDecision {
            reviewer: WorkerRole::SecurityReviewer,
            decision: ReviewDecision::Approve,
        }],
        rollback_target: "abc123".to_string(),
    }
}

/// Builds a proposal whose changed surface is exactly `paths`.
pub fn proposal_with_surface(paths: Vec<String>) -> phlow_experiment::ImprovementProposal {
    let mut params = proposal_params();
    params.changed_surface = paths;
    ok(phlow_experiment::ImprovementProposal::new(params))
}
