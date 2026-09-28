//! Orchestration tests: scheduler invariants as pure logic.
//!
//! Duplicate delivery, queue-full behavior, cancellation, and stale
//! generations — the invariants a future asynchronous executor must
//! preserve, verified here without any concurrency.

#[path = "common/mod.rs"]
mod common;

use common::{ok, test_node, test_scheduler};
use phlow_experiment::{ExperimentError, NodeId, NodeState, RunId, Scheduler, SchedulerLimits};

fn node_id(name: &str) -> NodeId {
    ok(NodeId::new(name))
}

#[test]
fn duplicate_result_delivery_at_most_once() {
    let mut scheduler = test_scheduler();
    ok(scheduler.admit(test_node("n1")));
    let id = node_id("n1");
    ok(scheduler.publish_result(&id, 0, "digest-1", NodeState::Succeeded));
    // The second delivery of the same node is rejected, not applied twice.
    let second = scheduler.publish_result(&id, 0, "digest-1", NodeState::Succeeded);
    assert!(matches!(
        second,
        Err(ExperimentError::DuplicateResult { .. })
    ));
    assert_eq!(scheduler.published_count(), 1);
}

#[test]
fn queue_full_rejects_explicitly() {
    let limits = SchedulerLimits {
        queue_capacity: 2,
        ..SchedulerLimits::default()
    };
    let mut scheduler = ok(Scheduler::new(limits));
    ok(scheduler.admit(test_node("n1")));
    ok(scheduler.admit(test_node("n2")));
    // Exact capacity is accepted; capacity + 1 is rejected explicitly —
    // never silently dropped.
    let overflow = scheduler.admit(test_node("n3"));
    assert!(matches!(overflow, Err(ExperimentError::QueueFull { .. })));
    assert_eq!(scheduler.queue_len(), 2);
}

#[test]
fn cancellation_is_terminal_and_blocks_publication() {
    let mut scheduler = test_scheduler();
    ok(scheduler.admit(test_node("n1")));
    ok(scheduler.admit(test_node("n2")));
    let run = ok(RunId::new("run-test-001"));
    let transitioned = scheduler.cancel_run(&run);
    assert_eq!(transitioned, 2);
    assert_eq!(scheduler.queue_len(), 0);
    for name in ["n1", "n2"] {
        let id = node_id(name);
        let node = match scheduler.node(&id) {
            Some(node) => node,
            None => panic!("admitted node {name} missing"),
        };
        assert_eq!(node.state(), NodeState::Cancelled);
        // Late results for a cancelled run are rejected.
        let late = scheduler.publish_result(&id, 0, "digest-late", NodeState::Succeeded);
        assert!(matches!(late, Err(ExperimentError::RunCancelled { .. })));
    }
    assert_eq!(scheduler.published_count(), 0);
}

#[test]
fn stale_generation_result_rejected() {
    let mut scheduler = test_scheduler();
    ok(scheduler.admit(test_node("n1")));
    let id = node_id("n1");
    // The node was admitted at generation 0; a result claiming generation 1
    // is stale and never publishes.
    let stale = scheduler.publish_result(&id, 1, "digest-1", NodeState::Succeeded);
    match stale {
        Err(ExperimentError::StaleGeneration { expected, got, .. }) => {
            assert_eq!(expected, 0);
            assert_eq!(got, 1);
        }
        other => panic!("expected StaleGeneration, got {other:?}"),
    }
    assert_eq!(scheduler.published_count(), 0);
    // The node is still live and can publish at its real generation.
    ok(scheduler.publish_result(&id, 0, "digest-1", NodeState::Succeeded));
    assert_eq!(scheduler.published_count(), 1);
}
