//! Validation tests: lifecycle paths, capability rules, state
//! classification, shipped-manifest parsing, and budget accounting.
//!
//! Every test states explicit inputs, the expected state transition, and
//! the expected result. Nothing here weakens a check to pass.

#[path = "common/mod.rs"]
mod common;

use common::{manifest_text, ok, root_capabilities, test_budget};
use phlow_experiment::{
    ExperimentError, Lifecycle, LifecycleEvent, NodeState, parse_budget_manifest,
    parse_language_manifest, parse_promotion_manifest, parse_suite_manifest,
};

#[test]
fn lifecycle_happy_path() {
    let state = ok(Lifecycle::Proposed.transition(LifecycleEvent::WorkspaceCreated));
    assert_eq!(state, Lifecycle::Isolated);
    let state = ok(state.transition(LifecycleEvent::ChecksComplete));
    assert_eq!(state, Lifecycle::Tested);
    let state = ok(state.transition(LifecycleEvent::EvaluationComplete));
    assert_eq!(state, Lifecycle::Reviewed);
    let state = ok(state.transition(LifecycleEvent::GatesSatisfied));
    assert_eq!(state, Lifecycle::AwaitingHuman);
    let state = ok(state.transition(LifecycleEvent::HumanApproved));
    assert_eq!(state, Lifecycle::Promoted);
    let state = ok(state.transition(LifecycleEvent::DeployedToCanary));
    assert_eq!(state, Lifecycle::Monitored);
}

#[test]
fn lifecycle_rejection_paths() {
    // Invalid contract or budget rejects from Proposed.
    let rejected = ok(Lifecycle::Proposed.transition(LifecycleEvent::ContractInvalid));
    assert_eq!(rejected, Lifecycle::Rejected);
    // Timeout, crash, or failed checks reject from Isolated.
    let rejected = ok(Lifecycle::Isolated.transition(LifecycleEvent::ChecksFailed));
    assert_eq!(rejected, Lifecycle::Rejected);
    // Regression or unsafe behavior rejects from Tested and Reviewed.
    let rejected = ok(Lifecycle::Tested.transition(LifecycleEvent::RegressionFound));
    assert_eq!(rejected, Lifecycle::Rejected);
    let rejected = ok(Lifecycle::Reviewed.transition(LifecycleEvent::RegressionFound));
    assert_eq!(rejected, Lifecycle::Rejected);
    // Denied or expired approval rejects from AwaitingHuman.
    let rejected = ok(Lifecycle::AwaitingHuman.transition(LifecycleEvent::ApprovalDenied));
    assert_eq!(rejected, Lifecycle::Rejected);
    let rejected = ok(Lifecycle::AwaitingHuman.transition(LifecycleEvent::ApprovalExpired));
    assert_eq!(rejected, Lifecycle::Rejected);
    // Regression or incident rolls back from Monitored.
    let rolled_back = ok(Lifecycle::Monitored.transition(LifecycleEvent::RegressionDetected));
    assert_eq!(rolled_back, Lifecycle::RolledBack);
}

#[test]
fn lifecycle_terminal_classification() {
    assert!(Lifecycle::Rejected.is_terminal());
    assert!(Lifecycle::RolledBack.is_terminal());
    for active in [
        Lifecycle::Proposed,
        Lifecycle::Isolated,
        Lifecycle::Tested,
        Lifecycle::Reviewed,
        Lifecycle::AwaitingHuman,
        Lifecycle::Promoted,
        Lifecycle::Monitored,
    ] {
        assert!(!active.is_terminal(), "{active:?} must not be terminal");
    }
}

#[test]
fn capability_strict_subset_accepted() {
    let parent = root_capabilities();
    let child = ok(parent.derive_child(
        vec!["read".to_string()],
        vec!["workspace/".to_string()],
        32,
        524_288,
    ));
    assert!(child.is_subset_of(&parent));
    assert_eq!(child.tools(), &["read".to_string()]);
    assert_eq!(child.tool_calls_max(), 32);
}

#[test]
fn capability_equal_set_rejected_as_not_strict() {
    // Delegation must shrink authority: re-labeling the parent's exact set
    // is not a strict subset and is rejected.
    let parent = root_capabilities();
    let result = parent.derive_child(
        vec!["read".to_string(), "exec-check".to_string()],
        vec!["workspace/".to_string(), "fixtures/".to_string()],
        64,
        1_048_576,
    );
    assert!(matches!(result, Err(ExperimentError::NotStrictSubset)));
}

#[test]
fn node_state_terminal_classification() {
    for terminal in [
        NodeState::Succeeded,
        NodeState::Failed,
        NodeState::Cancelled,
        NodeState::TimedOut,
        NodeState::Rejected,
        NodeState::Stale,
        NodeState::Superseded,
    ] {
        assert!(terminal.is_terminal(), "{terminal:?} must be terminal");
    }
    for active in [
        NodeState::Proposed,
        NodeState::Admitted,
        NodeState::Preparing,
        NodeState::Executing,
        NodeState::Verifying,
        NodeState::Reviewing,
    ] {
        assert!(!active.is_terminal(), "{active:?} must not be terminal");
    }
}

#[test]
fn manifest_suites_parses() {
    let manifest = ok(parse_suite_manifest(
        &manifest_text("suites.toml"),
        "manifests/suites.toml",
    ));
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.suites.len(), 6);
    let ids: Vec<&str> = manifest.suites.iter().map(|s| s.id.as_str()).collect();
    for expected in [
        "unit",
        "integration",
        "adversarial",
        "orchestration",
        "self_edit",
        "performance",
    ] {
        assert!(ids.contains(&expected), "missing suite {expected}");
    }
    assert!(manifest.suites.iter().all(|s| s.required));
}

#[test]
fn manifest_languages_parses() {
    let manifest = ok(parse_language_manifest(
        &manifest_text("languages.toml"),
        "manifests/languages.toml",
    ));
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.tiers.len(), 5);
    let tiers: Vec<char> = manifest.tiers.iter().map(|t| t.tier).collect();
    assert_eq!(tiers, vec!['A', 'B', 'C', 'D', 'E']);
    let tier_a = &manifest.tiers[0];
    let names: Vec<&str> = tier_a.languages.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, vec!["rust", "python", "lua"]);
    let rust = &tier_a.languages[0];
    assert!(rust.required_checks.contains(&"clippy".to_string()));
}

#[test]
fn manifest_budgets_parses() {
    let manifest = ok(parse_budget_manifest(
        &manifest_text("budgets.toml"),
        "manifests/budgets.toml",
    ));
    assert_eq!(manifest.schema_version, 1);
    let defaults = &manifest.defaults;
    assert_eq!(defaults.workers_max, 4);
    assert_eq!(defaults.queue_capacity, 16);
    assert_eq!(defaults.depth_max, 3);
    assert_eq!(defaults.task_deadline_ms, 300_000);
    assert_eq!(defaults.tool_calls_max, 64);
    assert_eq!(defaults.model_turns_max, 12);
}

#[test]
fn manifest_promotion_parses() {
    let manifest = ok(parse_promotion_manifest(
        &manifest_text("promotion.toml"),
        "manifests/promotion.toml",
    ));
    assert_eq!(manifest.schema_version, 1);
    let thresholds = &manifest.thresholds;
    assert_eq!(thresholds.critical_safety_pass_pct, 100);
    assert!(!thresholds.allow_new_failures_on_prior_successes);
    assert!(thresholds.hidden_holdout_gain_required);
    assert_eq!(thresholds.p95_latency_increase_pct_max, 10);
    assert_eq!(thresholds.cost_increase_pct_max, 20);
    assert!(thresholds.cost_increase_requires_approval);
    assert!(thresholds.rollback_rehearsal_required);
}

#[test]
fn budget_tracker_accounting() {
    let mut tracker = test_budget();
    ok(tracker.consume(10, 1024));
    assert_eq!(tracker.tool_calls_remaining(), 54);
    assert_eq!(tracker.output_bytes_used(), 1024);
    // A zero-cost consume is a no-op, not an error.
    ok(tracker.consume(0, 0));
    assert_eq!(tracker.tool_calls_remaining(), 54);
    assert_eq!(tracker.output_bytes_used(), 1024);
}
