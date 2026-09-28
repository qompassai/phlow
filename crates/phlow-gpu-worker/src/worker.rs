//! The worker: one job at a time, explicit states, generation cancellation.

use crate::error::WorkerError;
use crate::job::{Job, JobHandle};
use phlow_compute_cuda::{CudaBackend, CudaError, LaunchReceipt};

/// Observable worker state. [`Worker::state`] reports this; transitions
/// happen only through `submit`, `finish`, `cancel`, and `stop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    /// Ready to accept a job.
    Idle,
    /// A job has launched and its receipt waits to be collected.
    Busy {
        /// The running job's id.
        job_id: u64,
        /// The generation the job was submitted in.
        generation: u64,
        /// The backend's proof the launch was accepted.
        receipt: LaunchReceipt,
    },
    /// Terminal: the worker accepts nothing more.
    Stopped,
}

/// Dispatches [`Job`]s to a [`CudaBackend`] with a bounded lifecycle.
///
/// The worker runs one job at a time. `submit` launches the kernel on the
/// backend immediately (the simulated backend is synchronous) and holds the
/// resulting receipt until `finish` collects it. `cancel` drops the pending
/// result and invalidates outstanding handles by bumping the generation;
/// `stop` ends the worker permanently.
#[derive(Debug)]
pub struct Worker<B> {
    backend: B,
    state: WorkerState,
    generation: u64,
    next_job_id: u64,
}

impl<B: CudaBackend<Error = CudaError>> Worker<B> {
    /// Creates an idle worker around `backend`, at generation zero.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            state: WorkerState::Idle,
            generation: 0,
            next_job_id: 0,
        }
    }

    /// Reports the current lifecycle state.
    pub fn state(&self) -> WorkerState {
        self.state
    }

    /// Reports the current generation. It starts at zero and grows by one
    /// on every `cancel` and `stop`.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Gives callers direct access to the backend (for buffer setup).
    pub fn backend(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Submits a job: launches it on the backend and holds the receipt.
    ///
    /// # Contract
    /// - Accepts: worker is [`WorkerState::Idle`].
    /// - On backend failure the worker stays idle and the error surfaces
    ///   as [`WorkerError::Backend`]; no job id is consumed.
    /// - Rejects: busy workers ([`WorkerError::AlreadyRunning`]), stopped
    ///   workers ([`WorkerError::Stopped`]), exhausted job ids.
    pub fn submit(&mut self, job: Job<'_>) -> Result<JobHandle, WorkerError> {
        match self.state {
            WorkerState::Busy { job_id, .. } => {
                return Err(WorkerError::AlreadyRunning { job_id });
            }
            WorkerState::Stopped => return Err(WorkerError::Stopped),
            WorkerState::Idle => {}
        }
        let job_id = self.next_job_id;
        let receipt: LaunchReceipt = self
            .backend
            .launch(&job.descriptor, &job.config, job.args)?;
        self.next_job_id = job_id.checked_add(1).ok_or(WorkerError::JobIdExhausted)?;
        let generation = self.generation;
        self.state = WorkerState::Busy {
            job_id,
            generation,
            receipt,
        };
        Ok(JobHandle { job_id, generation })
    }

    /// Collects the running job's launch receipt and returns to idle.
    ///
    /// # Contract
    /// - Accepts: worker is busy and `handle` names the running job in the
    ///   current generation.
    /// - Rejects: idle ([`WorkerError::NotRunning`]), stopped
    ///   ([`WorkerError::Stopped`]), stale generations
    ///   ([`WorkerError::StaleHandle`]), mismatched job ids
    ///   ([`WorkerError::WrongJob`]).
    pub fn finish(&mut self, handle: JobHandle) -> Result<LaunchReceipt, WorkerError> {
        match self.state {
            WorkerState::Idle => Err(WorkerError::NotRunning),
            WorkerState::Stopped => Err(WorkerError::Stopped),
            WorkerState::Busy {
                job_id,
                generation,
                receipt,
            } => {
                if handle.generation != generation {
                    return Err(WorkerError::StaleHandle {
                        current: generation,
                        got: handle.generation,
                    });
                }
                if handle.job_id != job_id {
                    return Err(WorkerError::WrongJob {
                        running: job_id,
                        got: handle.job_id,
                    });
                }
                self.state = WorkerState::Idle;
                Ok(receipt)
            }
        }
    }

    /// Cancels the running job: drops its result and bumps the generation.
    ///
    /// # Contract
    /// - Accepts: worker is [`WorkerState::Busy`]; returns the cancelled
    ///   job's id.
    /// - Rejects: idle ([`WorkerError::NotRunning`]), stopped
    ///   ([`WorkerError::Stopped`]).
    pub fn cancel(&mut self) -> Result<u64, WorkerError> {
        match self.state {
            WorkerState::Busy { job_id, .. } => {
                self.state = WorkerState::Idle;
                self.generation = self.generation.saturating_add(1);
                Ok(job_id)
            }
            WorkerState::Idle => Err(WorkerError::NotRunning),
            WorkerState::Stopped => Err(WorkerError::Stopped),
        }
    }

    /// Stops the worker permanently: drops any running job and bumps the
    /// generation so outstanding handles go stale.
    pub fn stop(&mut self) {
        self.state = WorkerState::Stopped;
        self.generation = self.generation.saturating_add(1);
    }
}
