//! task-20: deadline expiry and at-most-once (rust).
//!
//! Drives phlow's real host-owned scheduler —
//! `phlow_experiment::control_plane::Scheduler`
//! (`crates/phlow-experiment/src/control_plane.rs`) — through the
//! deadline/at-most-once contract:
//!
//! - an expired deadline is published as the terminal state `TimedOut`
//!   (machine name `"timed_out"`, pairwise distinct from `"failed"` and
//!   `"cancelled"` so the operator can tell them apart at a glance),
//! - a late cancellation cannot resurrect a terminal node: `cancel_run`
//!   touches only non-terminal nodes, and a late result for a cancelled
//!   run is rejected with `RunCancelled`,
//! - results publish at most once: a second publication is rejected with
//!   `DuplicateResult` and the recorded digest never changes,
//! - stale generations and non-terminal states cannot publish
//!   (`StaleGeneration`, `NotTerminal`).
//!
//! Scope, stated plainly (same as task-13): phlow ships no executing
//! scheduler — the `Scheduler` "spawns nothing, runs nothing, and holds
//! no threads". Deadline *detection* (watching the clock) belongs to the
//! future executor; the invariant core tested here is what the host must
//! do once a deadline has expired: record `TimedOut` exactly once and let
//! nothing — late results, stale generations, late cancellations —
//! resurrect or double-complete the run.

use crate::{Ctx, TaskKind, TaskOutcome};
use phlow_experiment::{
    CapabilitySet, ExperimentError, ExperimentId, NodeId, NodeParams, NodeState, RunId, Scheduler,
    SchedulerLimits, SchedulerNode, WorkerRole,
};

/// Task id.
pub const ID: &str = "task-20";
/// Human-readable name.
pub const NAME: &str = "deadline expiry and at-most-once";
/// How this task is driven.
pub const KIND: TaskKind = TaskKind::Rust;

/// Build a scheduler with default limits.
fn make_scheduler() -> Scheduler {
    Scheduler::new(SchedulerLimits::default()).expect("task-20: default limits are valid")
}

fn make_ids(tag: &str) -> (RunId, ExperimentId) {
    let run = RunId::new(&format!("run-{tag}")).expect("task-20: valid run id");
    let exp = ExperimentId::new(&format!("exp-{tag}")).expect("task-20: valid experiment id");
    (run, exp)
}

fn make_capabilities() -> CapabilitySet {
    CapabilitySet::new(
        vec!["read".to_string()],
        vec!["workspace".to_string()],
        64,
        65_536,
    )
    .expect("task-20: valid capabilities")
}

/// Admit one node and return its id. Panics only on fixture bugs — the
/// driver asserts behavior, the fixtures are trusted setup.
fn admit_node(sched: &mut Scheduler, tag: &str, seq: usize) -> NodeId {
    let (run, exp) = make_ids(tag);
    let node_id = NodeId::new(&format!("node-{tag}-{seq}")).expect("task-20: valid node id");
    let node = SchedulerNode::new(NodeParams {
        run_id: run,
        experiment_id: exp,
        baseline_revision: "rev-1".to_string(),
        workspace_snapshot: "snap-1".to_string(),
        node_id: node_id.clone(),
        parent_node_id: None,
        role: WorkerRole::Implementer,
        capabilities: make_capabilities(),
        input_digest: format!("input-{tag}-{seq}"),
        dependency_ids: Vec::new(),
        generation: 0,
        attempt: 0,
        deadline_ms: 1_000,
        cpu_budget_ms: 1_000,
        memory_budget_bytes: 1_048_576,
        output_bytes_max: 4_096,
        tool_calls_remaining: 10,
    })
    .expect("task-20: valid node params");
    sched.admit(node).expect("task-20: node admits");
    node_id
}

/// The operator-facing verdict line for one node: the terminal state name
/// exactly as the operator reads it, with the at-a-glance distinctness
/// check baked in.
fn operator_view(state: NodeState) -> String {
    let others = [NodeState::Failed, NodeState::Cancelled, NodeState::TimedOut];
    let distinct = others
        .iter()
        .all(|other| *other == state || other.name() != state.name());
    format!(
        "operator view: verdict={} (distinct from failed/cancelled/timed_out peers: {distinct})",
        state.name()
    )
}

/// Attempt the task: deadline expiry, late-cancel, and at-most-once phases.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    let mut evidence = Vec::new();
    match drive(&mut evidence) {
        Ok(()) => TaskOutcome::Pass { evidence },
        Err((where_, how)) => TaskOutcome::Fail {
            where_,
            how,
            evidence,
        },
    }
}

fn drive(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    phase_deadline_expiry(evidence)?;
    phase_late_cancel_no_resurrect(evidence)?;
    phase_double_publish_rejected(evidence)?;
    phase_stale_and_nonterminal_rejected(evidence)?;
    Ok(())
}

/// V1: the deadline expires → the host publishes `TimedOut`; the node is
/// terminal, the digest is recorded exactly once, and the verdict name is
/// `timed_out` — not `failed`, not `cancelled`.
fn phase_deadline_expiry(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    let mut sched = make_scheduler();
    let id = admit_node(&mut sched, "deadline", 0);
    // The host observes the expired deadline and publishes the terminal
    // result: this is the whole deadline-expiry contract the invariant
    // core owns (detection belongs to the future executor).
    sched
        .publish_result(&id, 0, "digest-deadline-0", NodeState::TimedOut)
        .map_err(|err| {
            (
                "deadline-expiry".to_string(),
                format!("publishing the expired deadline failed: {err}"),
            )
        })?;
    let node = sched.node(&id).ok_or_else(|| {
        (
            "deadline-expiry".to_string(),
            "published node vanished".to_string(),
        )
    })?;
    if node.state() != NodeState::TimedOut {
        return Err((
            "deadline-expiry".to_string(),
            format!("node state is {}, want timed_out", node.state().name()),
        ));
    }
    if !node.state().is_terminal() {
        return Err((
            "deadline-expiry".to_string(),
            "timed_out is not terminal".to_string(),
        ));
    }
    if sched.published_digest(&id) != Some("digest-deadline-0") {
        return Err((
            "deadline-expiry".to_string(),
            "published digest not recorded".to_string(),
        ));
    }
    // At-a-glance distinctness: the three terminal names the operator
    // must never confuse.
    for (state, want) in [
        (NodeState::TimedOut, "timed_out"),
        (NodeState::Failed, "failed"),
        (NodeState::Cancelled, "cancelled"),
    ] {
        if state.name() != want {
            return Err((
                "deadline-expiry".to_string(),
                format!("NodeState name drift: got {}", state.name()),
            ));
        }
    }
    if NodeState::TimedOut.name() == NodeState::Failed.name()
        || NodeState::TimedOut.name() == NodeState::Cancelled.name()
    {
        return Err((
            "deadline-expiry".to_string(),
            "timed_out is not distinct from failed/cancelled".to_string(),
        ));
    }
    evidence.push(
        "ok: expired deadline published as timed_out (terminal, digest recorded once)".to_string(),
    );
    evidence.push(operator_view(NodeState::TimedOut));
    Ok(())
}

/// V2: a late cancellation cannot resurrect a terminal node, and a late
/// result for a cancelled run is rejected — the run id stays on the
/// cancelled list, the node keeps its terminal state.
fn phase_late_cancel_no_resurrect(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    let mut sched = make_scheduler();
    // A timed-out node, then a late cancel for its run: nothing moves.
    let timed_out_id = admit_node(&mut sched, "latecancel", 0);
    sched
        .publish_result(&timed_out_id, 0, "digest-timed-out", NodeState::TimedOut)
        .map_err(|err| {
            (
                "late-cancel".to_string(),
                format!("setup publish failed: {err}"),
            )
        })?;
    let run_of = |sched: &Scheduler, id: &NodeId| sched.node(id).unwrap().run_id().clone();
    let run_id = run_of(&sched, &timed_out_id);
    let transitioned = sched.cancel_run(&run_id);
    if transitioned != 0 {
        return Err((
            "late-cancel".to_string(),
            format!("cancel_run transitioned {transitioned} terminal nodes"),
        ));
    }
    let state = sched.node(&timed_out_id).unwrap().state();
    if state != NodeState::TimedOut {
        return Err((
            "late-cancel".to_string(),
            format!("late cancel resurrected the node: {}", state.name()),
        ));
    }
    evidence.push(
        "ok: cancel_run on a timed_out run transitioned 0 nodes; node still timed_out".to_string(),
    );

    // A live node whose run is cancelled, then a late result arrives: the
    // result is rejected and the node keeps its cancelled state. This needs
    // its own fresh run id: the "latecancel" run above is already cancelled,
    // and admitting any node under a cancelled run is itself rejected.
    let live_id = admit_node(&mut sched, "late-result", 1);
    let live_run = run_of(&sched, &live_id);
    let transitioned = sched.cancel_run(&live_run);
    if transitioned != 1 {
        return Err((
            "late-cancel".to_string(),
            format!("cancel_run transitioned {transitioned} live nodes, want 1"),
        ));
    }
    match sched.publish_result(&live_id, 0, "digest-late", NodeState::Succeeded) {
        Err(ExperimentError::RunCancelled { .. }) => {
            evidence
                .push("ok: late result for a cancelled run rejected with RunCancelled".to_string());
        }
        other => {
            return Err((
                "late-cancel".to_string(),
                format!("late result was not rejected with RunCancelled: {other:?}"),
            ));
        }
    }
    let state = sched.node(&live_id).unwrap().state();
    if state != NodeState::Cancelled {
        return Err((
            "late-cancel".to_string(),
            format!(
                "cancelled node changed state on late result: {}",
                state.name()
            ),
        ));
    }
    evidence.push("ok: cancelled node kept its state after the rejected late result".to_string());
    evidence.push(operator_view(NodeState::Cancelled));
    Ok(())
}

/// A1: at-most-once — the second publication is rejected with
/// `DuplicateResult`; the recorded digest and generation never change.
fn phase_double_publish_rejected(evidence: &mut Vec<String>) -> Result<(), (String, String)> {
    let mut sched = make_scheduler();
    let id = admit_node(&mut sched, "double", 0);
    sched
        .publish_result(&id, 0, "digest-first", NodeState::Succeeded)
        .map_err(|err| {
            (
                "double-publish".to_string(),
                format!("first publish failed: {err}"),
            )
        })?;
    match sched.publish_result(&id, 0, "digest-second", NodeState::Succeeded) {
        Err(ExperimentError::DuplicateResult { .. }) => {
            evidence.push("ok: second publication rejected with DuplicateResult".to_string());
        }
        other => {
            return Err((
                "double-publish".to_string(),
                format!("second publish was not rejected with DuplicateResult: {other:?}"),
            ));
        }
    }
    if sched.published_count() != 1 {
        return Err((
            "double-publish".to_string(),
            format!("published_count is {}", sched.published_count()),
        ));
    }
    if sched.published_digest(&id) != Some("digest-first") {
        return Err((
            "double-publish".to_string(),
            "recorded digest changed after the rejected duplicate".to_string(),
        ));
    }
    if sched.published_generation(&id) != Some(0) {
        return Err((
            "double-publish".to_string(),
            "recorded generation changed after the rejected duplicate".to_string(),
        ));
    }
    let state = sched.node(&id).unwrap().state();
    if state != NodeState::Succeeded {
        return Err((
            "double-publish".to_string(),
            format!("node state moved on duplicate publish: {}", state.name()),
        ));
    }
    evidence
        .push("ok: digest and generation unchanged; node still succeeded exactly once".to_string());
    Ok(())
}

/// A2: stale generations and non-terminal states cannot publish — the
/// generation check and the terminal check both fire before any mutation.
fn phase_stale_and_nonterminal_rejected(
    evidence: &mut Vec<String>,
) -> Result<(), (String, String)> {
    let mut sched = make_scheduler();
    let id = admit_node(&mut sched, "stale", 0);
    // Stale generation: the node is generation 0, the result claims 1.
    match sched.publish_result(&id, 1, "digest-stale", NodeState::Succeeded) {
        Err(ExperimentError::StaleGeneration { .. }) => {
            evidence.push("ok: stale-generation publish rejected with StaleGeneration".to_string());
        }
        other => {
            return Err((
                "stale-generation".to_string(),
                format!("stale publish was not rejected: {other:?}"),
            ));
        }
    }
    // Non-terminal state cannot publish at all.
    match sched.publish_result(&id, 0, "digest-nonterminal", NodeState::Executing) {
        Err(ExperimentError::NotTerminal { .. }) => {
            evidence.push("ok: non-terminal publish rejected with NotTerminal".to_string());
        }
        other => {
            return Err((
                "stale-generation".to_string(),
                format!("non-terminal publish was not rejected: {other:?}"),
            ));
        }
    }
    // Neither rejection mutated anything: the node is still admitted and
    // nothing was published.
    let node = sched.node(&id).unwrap();
    if node.state() != NodeState::Admitted {
        return Err((
            "stale-generation".to_string(),
            format!(
                "rejected publishes moved the node to {}",
                node.state().name()
            ),
        ));
    }
    if sched.published_count() != 0 {
        return Err((
            "stale-generation".to_string(),
            "a rejected publish recorded a result".to_string(),
        ));
    }
    evidence.push("ok: node still admitted, published_count 0 after both rejections".to_string());
    Ok(())
}
