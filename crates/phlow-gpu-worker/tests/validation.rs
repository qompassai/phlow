//! Validation tests: the worker lifecycle behaves as documented.

use phlow_compute_cuda::{
    ComputeArch, Dim3, EntryName, KernelDescriptor, KernelName, KernelPolicy, KernelSource,
    LaunchArgs, LaunchConfig, PtxModule, SimulatedBackend,
};
use phlow_gpu_worker::{Job, Worker, WorkerState};

const VALID_PTX: &str = include_str!("../../../kernels/ptx/valid_add.ptx");

fn noop(
    _buffers: &mut [Vec<u8>],
    _launch: &phlow_compute_cuda::ValidatedLaunch,
) -> Result<(), phlow_compute_cuda::BehaviorError> {
    Ok(())
}

fn worker() -> Worker<SimulatedBackend> {
    let mut backend = SimulatedBackend::new();
    backend.register_behavior("vector_add", noop);
    Worker::new(backend)
}

fn descriptor() -> KernelDescriptor {
    KernelDescriptor::new(
        KernelName::new("vector_add").expect("valid name"),
        EntryName::new("vector_add_kernel").expect("valid entry"),
        ComputeArch::Sm80,
        KernelSource::PtxText(PtxModule::from_text(VALID_PTX).expect("valid PTX")),
    )
}

fn launch_1d() -> LaunchConfig {
    LaunchConfig {
        grid: Dim3::new(1, 1, 1),
        block: Dim3::new(256, 1, 1),
        shared_mem_bytes: 0,
    }
}

fn empty_args() -> LaunchArgs {
    LaunchArgs::new(Vec::new(), Vec::new()).expect("empty args are valid")
}

fn job<'a>(args: &'a LaunchArgs) -> Job<'a> {
    Job::new(descriptor(), launch_1d(), args)
}

#[test]
fn new_worker_is_idle_at_generation_zero() {
    let worker = worker();
    assert_eq!(worker.state(), WorkerState::Idle);
    assert_eq!(worker.generation(), 0);
}

#[test]
fn submit_runs_and_returns_handle() {
    let mut worker = worker();
    let args = empty_args();
    let handle = worker.submit(job(&args)).expect("submit");
    assert_eq!(handle.job_id, 0);
    assert_eq!(handle.generation, 0);
    assert!(matches!(
        worker.state(),
        WorkerState::Busy { job_id: 0, .. }
    ));
}

#[test]
fn finish_returns_receipt_and_idles() {
    let mut worker = worker();
    let args = empty_args();
    let handle = worker.submit(job(&args)).expect("submit");
    let receipt = worker.finish(handle).expect("finish");
    assert_eq!(receipt.id, 1);
    assert_eq!(worker.state(), WorkerState::Idle);
}

#[test]
fn sequential_jobs_each_complete_with_new_ids() {
    let mut worker = worker();
    for expected_job in 0..3u64 {
        let args = empty_args();
        let handle = worker.submit(job(&args)).expect("submit");
        assert_eq!(handle.job_id, expected_job);
        let receipt = worker.finish(handle).expect("finish");
        assert_eq!(receipt.id, expected_job + 1);
    }
    assert_eq!(worker.state(), WorkerState::Idle);
}

#[test]
fn cancel_from_busy_returns_to_idle() {
    let mut worker = worker();
    let args = empty_args();
    worker.submit(job(&args)).expect("submit");
    let cancelled = worker.cancel().expect("cancel");
    assert_eq!(cancelled, 0);
    assert_eq!(worker.state(), WorkerState::Idle);
}

#[test]
fn cancel_bumps_generation() {
    let mut worker = worker();
    let args = empty_args();
    worker.submit(job(&args)).expect("submit");
    worker.cancel().expect("cancel");
    assert_eq!(worker.generation(), 1);
    // The worker is usable again at the new generation.
    let args2 = empty_args();
    let handle = worker.submit(job(&args2)).expect("submit after cancel");
    assert_eq!(handle.generation, 1);
    assert_eq!(handle.job_id, 1);
    worker.finish(handle).expect("finish");
}

#[test]
fn policy_flows_through_job() {
    let mut worker = worker();
    let args = empty_args();
    let policy = KernelPolicy::new(16, 4, true).expect("valid policy");
    let handle = worker
        .submit(job(&args).with_policy(policy))
        .expect("submit with policy");
    worker.finish(handle).expect("finish");
}

#[test]
fn job_with_scalar_args_completes() {
    // The simulator's behaviors see buffers, not scalar words; this test
    // pins the plumbing — scalar words travel through submit to launch.
    let mut worker = worker();
    let args = LaunchArgs::new(Vec::new(), vec![7, 8]).expect("scalar words are valid");
    let handle = worker.submit(job(&args)).expect("submit");
    let receipt = worker.finish(handle).expect("finish");
    assert_eq!(receipt.id, 1);
}

#[test]
fn stop_from_idle_is_terminal() {
    let mut worker = worker();
    worker.stop();
    assert_eq!(worker.state(), WorkerState::Stopped);
    assert_eq!(worker.generation(), 1);
}

#[test]
fn stop_from_busy_discards_job() {
    let mut worker = worker();
    let args = empty_args();
    worker.submit(job(&args)).expect("submit");
    worker.stop();
    assert_eq!(worker.state(), WorkerState::Stopped);
}
