//! Jobs and the handles that claim their results.

use phlow_compute_cuda::{KernelDescriptor, KernelPolicy, LaunchArgs, LaunchConfig};

/// One unit of GPU work: what to run and how to run it.
///
/// The launch arguments are borrowed from the caller and must outlive the
/// job; the worker never copies or retains them.
#[derive(Debug)]
pub struct Job<'a> {
    /// The kernel to launch.
    pub descriptor: KernelDescriptor,
    /// Grid/block geometry and shared-memory request.
    pub config: LaunchConfig,
    /// Device pointers and scalar words for the launch.
    pub args: &'a LaunchArgs,
    /// Optional compile-time specialization for the kernel.
    pub policy: Option<KernelPolicy>,
}

impl<'a> Job<'a> {
    /// Builds a job without a specialization policy.
    pub fn new(descriptor: KernelDescriptor, config: LaunchConfig, args: &'a LaunchArgs) -> Self {
        Self {
            descriptor,
            config,
            args,
            policy: None,
        }
    }

    /// Attaches a compile-time specialization policy to the job.
    #[must_use]
    pub fn with_policy(mut self, policy: KernelPolicy) -> Self {
        self.policy = Some(policy);
        self
    }
}

/// Proof that a job was submitted, and the key to collect its result.
///
/// A handle is only valid for the generation it was issued in. Cancelling
/// or stopping the worker invalidates every outstanding handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobHandle {
    /// The submitted job's id.
    pub job_id: u64,
    /// The worker generation the job was submitted in.
    pub generation: u64,
}
