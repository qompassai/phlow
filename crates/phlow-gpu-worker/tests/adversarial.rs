//! Adversarial tests: lifecycle misuse is rejected with typed errors —
//! never panics, never lost receipts, never stale handles accepted.

use phlow_compute_cuda::{
    BehaviorError, ComputeArch, CudaError, Dim3, EntryName, KernelDescriptor, KernelName,
    KernelSource, LaunchArgs, LaunchConfig, PtxModule, SimulatedBackend, ValidatedLaunch,
};
use phlow_gpu_worker::{Job, JobHandle, Worker, WorkerError, WorkerState};

const VALID_PTX: &str = include_str!("../../../kernels/ptx/valid_add.ptx");

fn noop(_buffers: &mut [Vec<u8>], _launch: &ValidatedLaunch) -> Result<(), BehaviorError> {
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

fn job(args: &LaunchArgs) -> Job<'_> {
    Job::new(descriptor(), launch_1d(), args)
}

#[test]
fn submit_while_busy_is_rejected_without_consuming_job_id() {
    let mut worker = worker();
    let args = empty_args();
    worker.submit(job(&args)).expect("first submit");
    let err = worker.submit(job(&args)).expect_err("second submit");
    assert_eq!(err, WorkerError::AlreadyRunning { job_id: 0 });
    // The rejected submit consumed nothing: after cancel the next job is 1.
    worker.cancel().expect("cancel");
    let args = empty_args();
    let handle = worker.submit(job(&args)).expect("submit after cancel");
    assert_eq!(handle.job_id, 1);
}

#[test]
fn finish_with_wrong_job_id_is_rejected() {
    let mut worker = worker();
    let args = empty_args();
    let handle = worker.submit(job(&args)).expect("submit");
    let forged = JobHandle {
        job_id: 999,
        generation: 0,
    };
    assert_eq!(
        worker.finish(forged),
        Err(WorkerError::WrongJob {
            running: 0,
            got: 999
        })
    );
    // The real job is untouched and still finishable.
    worker.finish(handle).expect("finish");
}

#[test]
fn finish_with_stale_generation_is_rejected() {
    let mut worker = worker();
    let args = empty_args();
    let old = worker.submit(job(&args)).expect("submit");
    worker.cancel().expect("cancel");
    let args = empty_args();
    let current = worker.submit(job(&args)).expect("submit again");
    assert_eq!(old.generation + 1, current.generation);
    assert_eq!(
        worker.finish(old),
        Err(WorkerError::StaleHandle { current: 1, got: 0 })
    );
    // The current job still finishes fine.
    worker.finish(current).expect("finish");
}

#[test]
fn finish_when_idle_is_rejected() {
    let mut worker = worker();
    let handle = JobHandle {
        job_id: 0,
        generation: 0,
    };
    assert_eq!(worker.finish(handle), Err(WorkerError::NotRunning));
}

#[test]
fn cancel_when_idle_is_rejected() {
    let mut worker = worker();
    assert_eq!(worker.cancel(), Err(WorkerError::NotRunning));
}

#[test]
fn submit_after_stop_is_rejected() {
    let mut worker = worker();
    worker.stop();
    let args = empty_args();
    assert_eq!(worker.submit(job(&args)), Err(WorkerError::Stopped));
}

#[test]
fn finish_after_stop_is_rejected() {
    let mut worker = worker();
    let args = empty_args();
    let handle = worker.submit(job(&args)).expect("submit");
    worker.stop();
    assert_eq!(worker.finish(handle), Err(WorkerError::Stopped));
}

#[test]
fn backend_failure_surfaces_and_worker_recovers() {
    fn failing(_buffers: &mut [Vec<u8>], _launch: &ValidatedLaunch) -> Result<(), BehaviorError> {
        Err(BehaviorError {
            detail: "injected failure",
        })
    }
    let mut backend = SimulatedBackend::new();
    backend.register_behavior("vector_add", failing);
    let mut worker = Worker::new(backend);
    let args = empty_args();
    let err = worker.submit(job(&args)).expect_err("backend fails");
    assert!(matches!(err, WorkerError::Backend(_)));
    // The worker stayed idle and the failed submit consumed no job id.
    assert_eq!(worker.state(), WorkerState::Idle);
    assert_eq!(worker.generation(), 0);
}

#[test]
fn unknown_kernel_surfaces_typed_backend_error() {
    let mut worker = worker();
    let descriptor = KernelDescriptor::new(
        KernelName::new("ghost").expect("valid name"),
        EntryName::new("ghost_kernel").expect("valid entry"),
        ComputeArch::Sm80,
        KernelSource::PtxText(PtxModule::from_text(VALID_PTX).expect("valid PTX")),
    );
    let args = empty_args();
    let job = Job::new(descriptor, launch_1d(), &args);
    let err = worker.submit(job).expect_err("unknown kernel");
    assert_eq!(
        err,
        WorkerError::Backend(CudaError::UnknownKernel {
            name: "ghost".to_string()
        })
    );
    assert_eq!(worker.state(), WorkerState::Idle);
}

#[test]
fn double_cancel_is_rejected() {
    let mut worker = worker();
    let args = empty_args();
    worker.submit(job(&args)).expect("submit");
    worker.cancel().expect("first cancel");
    assert_eq!(worker.cancel(), Err(WorkerError::NotRunning));
}
