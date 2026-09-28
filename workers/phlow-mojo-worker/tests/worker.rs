//! Integration tests for `phlow-mojo-worker`.
//!
//! Inventory: 20 tests, exactly 10 validation (v_*) and 10 adversarial
//! (a_*). Validation tests prove the harness contract; adversarial tests
//! throw hostile inputs, boundary violations, cancellation races, and
//! oversized payloads at it, asserting typed rejection and unchanged state.

use std::time::Duration;

use phlow_mojo_worker::types::TaskId;
use phlow_mojo_worker::{
    DrainReport, HarnessState, MojoWorker, PollOutcome, PumpReport, RESULTS_MAX,
    SimulatedMojoWorker, TaskKind, TaskStatus, WorkerConfig, WorkerError, WorkerHarness,
    WorkerResult, WorkerTask,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn config() -> WorkerConfig {
    WorkerConfig::new(128, 1024, Duration::from_secs(60)).unwrap()
}

fn running_harness() -> WorkerHarness<SimulatedMojoWorker> {
    let mut harness = WorkerHarness::new(SimulatedMojoWorker::new(), config());
    harness.start().unwrap();
    harness
}

/// A dishonest worker: returns `Ready` with a forged generation the harness
/// never issued. The harness must drop the result as stale.
struct LyingWorker;

impl MojoWorker for LyingWorker {
    fn init(&mut self, _config: &WorkerConfig) -> Result<(), WorkerError> {
        Ok(())
    }

    fn poll(&mut self, task: &WorkerTask) -> Result<PollOutcome, WorkerError> {
        let forged = WorkerResult::new(
            task.id(),
            task.generation().wrapping_add(999),
            TaskStatus::Completed,
            vec![1, 2, 3],
        )?;
        Ok(PollOutcome::Ready(forged))
    }

    fn cancel(&mut self, _id: TaskId, _generation: u64) {}

    fn shutdown(&mut self) {}
}

// ---------------------------------------------------------------------------
// Validation (10)
// ---------------------------------------------------------------------------

#[test]
fn v_start_stop_lifecycle() {
    let mut harness = WorkerHarness::new(SimulatedMojoWorker::new(), config());
    assert_eq!(harness.state(), HarnessState::Stopped);
    harness.start().unwrap();
    assert_eq!(harness.state(), HarnessState::Running);
    let report: DrainReport = harness.drain().unwrap();
    assert_eq!(report.completed, 0);
    assert_eq!(harness.state(), HarnessState::Stopped);
    assert!(harness.worker().shutdown_called());
}

#[test]
fn v_submit_accepted_while_running() {
    let mut harness = running_harness();
    let id = harness
        .submit(TaskKind::Transform, b"hello".to_vec())
        .unwrap();
    assert_eq!(id.get(), 0);
    assert_eq!(harness.queue_len(), 1);
}

#[test]
fn v_result_matches_task_id_and_generation() {
    let mut harness = running_harness();
    let id = harness
        .submit(TaskKind::Transform, b"abc".to_vec())
        .unwrap();
    let report: PumpReport = harness.pump().unwrap();
    assert_eq!(report.admitted, 1);
    assert_eq!(report.completed, 1);
    let result = harness.take_result().unwrap();
    assert_eq!(result.task_id(), id);
    assert_eq!(result.generation(), 0);
    assert_eq!(result.status(), &TaskStatus::Completed);
    // Simulated transform: each byte wrapping-adds 1.
    assert_eq!(result.output(), b"bcd");
    assert!(harness.take_result().is_none());
}

#[test]
fn v_cancel_queued_task_publishes_cancelled() {
    let mut harness = running_harness();
    let id = harness.submit(TaskKind::Embed, b"data".to_vec()).unwrap();
    harness.cancel(id).unwrap();
    assert_eq!(harness.queue_len(), 0);
    let result = harness.take_result().unwrap();
    assert_eq!(result.task_id(), id);
    assert_eq!(result.status(), &TaskStatus::Cancelled);
}

#[test]
fn v_queue_full_rejects_with_typed_error() {
    let tight = WorkerConfig::new(2, 1024, Duration::from_secs(60)).unwrap();
    let mut harness = WorkerHarness::new(SimulatedMojoWorker::new(), tight);
    harness.start().unwrap();
    harness.submit(TaskKind::Score, vec![1]).unwrap();
    harness.submit(TaskKind::Score, vec![2]).unwrap();
    assert_eq!(
        harness.submit(TaskKind::Score, vec![3]),
        Err(WorkerError::QueueFull { capacity: 2 })
    );
    assert_eq!(harness.queue_len(), 2);
}

#[test]
fn v_drain_processes_remaining_then_stops() {
    let mut harness = running_harness();
    for i in 0..3u8 {
        harness.submit(TaskKind::Transform, vec![i]).unwrap();
    }
    let report = harness.drain().unwrap();
    assert_eq!(report.completed, 3);
    assert_eq!(harness.state(), HarnessState::Stopped);
    assert_eq!(harness.results_len(), 3);
}

#[test]
fn v_restart_bumps_generation() {
    let mut harness = running_harness();
    harness.submit(TaskKind::Score, vec![9]).unwrap();
    harness.pump().unwrap();
    // Consume the generation-0 result first: results survive restart, so the
    // stale entry must be drained before the new generation's result.
    let stale = harness.take_result().unwrap();
    assert_eq!(stale.generation(), 0);
    harness.restart().unwrap();
    assert_eq!(harness.generation(), 1);
    assert_eq!(harness.state(), HarnessState::Running);
    let id = harness.submit(TaskKind::Score, vec![9]).unwrap();
    harness.pump().unwrap();
    let result = harness.take_result().unwrap();
    assert_eq!(result.task_id(), id);
    assert_eq!(result.generation(), 1);
}

#[test]
fn v_payload_exact_limit_accepted() {
    let mut harness = running_harness();
    let payload = vec![0xabu8; 1024];
    let id = harness.submit(TaskKind::Embed, payload).unwrap();
    assert_eq!(id.get(), 0);
}

#[test]
fn v_simulated_worker_completes_transform() {
    let mut harness = WorkerHarness::new(SimulatedMojoWorker::with_pending_polls(2), config());
    harness.start().unwrap();
    harness.submit(TaskKind::Transform, vec![10, 20]).unwrap();
    // Two pending polls, then completion on the third pump.
    for _ in 0..2 {
        let report = harness.pump().unwrap();
        assert_eq!(report.completed, 0);
    }
    let report = harness.pump().unwrap();
    assert_eq!(report.completed, 1);
    let result = harness.take_result().unwrap();
    assert_eq!(result.output(), &[11, 21]);
}

#[test]
fn v_init_failure_returns_to_stopped() {
    let mut worker = SimulatedMojoWorker::new();
    worker.fail_next_init();
    let mut harness = WorkerHarness::new(worker, config());
    let result = harness.start();
    assert!(matches!(result, Err(WorkerError::InitFailed { .. })));
    assert_eq!(harness.state(), HarnessState::Stopped);
}

// ---------------------------------------------------------------------------
// Adversarial (10)
// ---------------------------------------------------------------------------

#[test]
fn a_submit_while_stopped_rejected() {
    let mut harness = WorkerHarness::new(SimulatedMojoWorker::new(), config());
    assert_eq!(
        harness.submit(TaskKind::Transform, b"x".to_vec()),
        Err(WorkerError::NotRunning)
    );
    assert_eq!(harness.queue_len(), 0);
}

#[test]
fn a_oversized_payload_rejected_before_queue() {
    let mut harness = running_harness();
    let payload = vec![0u8; 1025];
    assert_eq!(
        harness.submit(TaskKind::Embed, payload),
        Err(WorkerError::PayloadTooLarge {
            bytes: 1025,
            max: 1024,
        })
    );
    // Rejected before admission: the queue is untouched.
    assert_eq!(harness.queue_len(), 0);
}

#[test]
fn a_double_cancel_rejected() {
    let mut harness = running_harness();
    let id = harness.submit(TaskKind::Score, vec![1]).unwrap();
    harness.cancel(id).unwrap();
    // Already cancelled and removed: the second cancel finds nothing, and
    // the first cancellation published exactly one result (no duplicates).
    assert!(matches!(
        harness.cancel(id),
        Err(WorkerError::UnknownTask { .. })
    ));
    assert_eq!(harness.results_len(), 1);
}

#[test]
fn a_cancel_completed_task_is_not_resurrected() {
    let mut harness = running_harness();
    let id = harness.submit(TaskKind::Transform, vec![7]).unwrap();
    harness.pump().unwrap();
    // Completed: exactly one result exists, and cancellation is refused.
    assert!(matches!(
        harness.cancel(id),
        Err(WorkerError::UnknownTask { .. })
    ));
    assert_eq!(harness.results_len(), 1);
    let result = harness.take_result().unwrap();
    assert_eq!(result.status(), &TaskStatus::Completed);
}

#[test]
fn a_stale_generation_result_dropped() {
    let mut harness = WorkerHarness::new(LyingWorker, config());
    harness.start().unwrap();
    harness.submit(TaskKind::Score, vec![1]).unwrap();
    let report = harness.pump().unwrap();
    // The forged result is dropped, counted, and never published.
    assert_eq!(report.stale_dropped, 1);
    assert_eq!(harness.stale_results_dropped(), 1);
    assert!(harness.take_result().is_none());
    assert_eq!(harness.inflight_len(), 0);
}

#[test]
fn a_poll_failure_keeps_task() {
    let mut worker = SimulatedMojoWorker::new();
    worker.fail_next_poll();
    let mut harness = WorkerHarness::new(worker, config());
    harness.start().unwrap();
    harness.submit(TaskKind::Transform, vec![5]).unwrap();
    let result = harness.pump();
    assert!(result.is_err());
    // The failed admission kept the task: nothing lost, nothing published.
    assert_eq!(harness.queue_len(), 1);
    assert_eq!(harness.inflight_len(), 0);
    assert_eq!(harness.results_len(), 0);
    // The next pump succeeds: the task was not corrupted by the failure.
    let report = harness.pump().unwrap();
    assert_eq!(report.completed, 1);
}

#[test]
fn a_results_overflow_drops_oldest_counts() {
    let mut harness = running_harness();
    for i in 0..=RESULTS_MAX as u8 {
        harness.submit(TaskKind::Transform, vec![i]).unwrap();
    }
    harness.pump().unwrap();
    harness.pump().unwrap();
    // 65 results into 64 slots: the oldest is dropped, visibly counted.
    assert_eq!(harness.results_dropped(), 1);
    assert_eq!(harness.results_len(), RESULTS_MAX);
    let first = harness.take_result().unwrap();
    assert_eq!(first.task_id().get(), 1); // task-0 was dropped
}

#[test]
fn a_double_start_rejected() {
    let mut harness = running_harness();
    assert_eq!(harness.start(), Err(WorkerError::AlreadyRunning));
    assert_eq!(harness.state(), HarnessState::Running);
}

#[test]
fn a_cancel_inflight_marks_cancelled() {
    let mut harness = WorkerHarness::new(SimulatedMojoWorker::with_pending_polls(8), config());
    harness.start().unwrap();
    let id = harness.submit(TaskKind::Embed, b"slow".to_vec()).unwrap();
    harness.pump().unwrap();
    assert_eq!(harness.inflight_len(), 1);
    // Cancel while the worker still reports Pending: the harness marks it
    // and notifies the worker; the next pump publishes Cancelled.
    harness.cancel(id).unwrap();
    let report = harness.pump().unwrap();
    assert_eq!(report.cancelled, 1);
    let result = harness.take_result().unwrap();
    assert_eq!(result.task_id(), id);
    assert_eq!(result.status(), &TaskStatus::Cancelled);
}

#[test]
fn a_zero_queue_capacity_rejected_at_config() {
    assert!(matches!(
        WorkerConfig::new(0, 1024, Duration::from_secs(60)),
        Err(WorkerError::InvalidConfig { .. })
    ));
    assert!(matches!(
        WorkerConfig::new(128, 0, Duration::from_secs(60)),
        Err(WorkerError::InvalidConfig { .. })
    ));
    assert!(matches!(
        WorkerConfig::new(128, 1024, Duration::ZERO),
        Err(WorkerError::InvalidConfig { .. })
    ));
}
