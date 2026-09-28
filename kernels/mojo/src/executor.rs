//! The executor boundary and the simulated executor.
//!
//! [`KernelExecutor`] is the interface a real device backend implements: it
//! takes a [`LaunchPlan`] (validated launches only — unvalidated launches are
//! unrepresentable at this boundary) and returns a [`LaunchReceipt`]. The
//! two-step shape mirrors Mojo's `DeviceContext`: `launch` enqueues the
//! kernel the way `enqueue_function` does (asynchronous, returns once
//! queued), and `synchronize` blocks until the device drains its queue the
//! way `DeviceContext.synchronize()` does.
//!
//! The `simulated` feature provides [`SimulatedExecutor`], an in-process
//! test double. It performs no GPU work and compiles no Mojo; it verifies
//! the plan is well-formed (re-deriving the thread product with checked
//! arithmetic) and returns a deterministic receipt so the contract is
//! exercisable without hardware. It is never a substitute for a device.

use crate::budget::LaunchPlan;
use crate::error::KernelError;

/// Maximum characters in an executor rejection reason.
pub const REASON_CHARS_MAX: usize = 256;

/// What a real device executor must implement.
///
/// Ownership: the executor owns the device queue it enqueues into; the plan
/// is borrowed for inspection only. Cancellation of an enqueued launch is a
/// device-specific policy documented by the implementing backend, not here.
pub trait KernelExecutor {
    /// Enqueue the plan's kernel on the device (asynchronous, like Mojo's
    /// `enqueue_function`).
    ///
    /// # Errors
    ///
    /// [`KernelError::ExecutorRejected`] when the device refuses the
    /// well-formed plan. The plan itself stays valid; the caller may retry
    /// against a different executor.
    fn launch(&mut self, plan: &LaunchPlan) -> Result<LaunchReceipt, KernelError>;

    /// Block until the device completes all enqueued operations (like
    /// Mojo's `DeviceContext.synchronize()`).
    ///
    /// # Errors
    ///
    /// [`KernelError::ExecutorRejected`] on device-side failure.
    fn synchronize(&mut self) -> Result<(), KernelError>;
}

/// Proof that an executor accepted a launch plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchReceipt {
    plan_sequence: u64,
    kernel: String,
    total_threads: u64,
    checksum: u64,
}

impl LaunchReceipt {
    /// The plan sequence this receipt answers.
    #[must_use]
    pub fn plan_sequence(&self) -> u64 {
        self.plan_sequence
    }

    /// Kernel name the receipt covers.
    #[must_use]
    pub fn kernel(&self) -> &str {
        &self.kernel
    }

    /// Total threads the receipt covers.
    #[must_use]
    pub fn total_threads(&self) -> u64 {
        self.total_threads
    }

    /// Deterministic checksum over the plan (simulated executor only).
    #[must_use]
    pub fn checksum(&self) -> u64 {
        self.checksum
    }
}

/// Bound a rejection reason to [`REASON_CHARS_MAX`] characters.
fn bound_reason(reason: &str) -> String {
    reason.chars().take(REASON_CHARS_MAX).collect()
}

/// FNV-1a 64-bit hash over bytes. Deterministic across runs (unlike
/// `DefaultHasher`), so simulated receipts are reproducible in tests.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// In-process test double for [`KernelExecutor`] (`simulated` feature).
///
/// Re-derives the launch's thread product with checked arithmetic
/// (defense in depth: the type already guarantees validation, but the
/// executor re-checks the arithmetic behind the numbers it reports), then
/// returns a receipt whose checksum is a deterministic FNV-1a hash over the
/// kernel name and launch geometry. No threads are spawned, no device is
/// touched, no Mojo is compiled.
#[cfg(feature = "simulated")]
#[derive(Debug, Default)]
pub struct SimulatedExecutor {
    launches: u64,
    fail_next: Option<String>,
}

#[cfg(feature = "simulated")]
impl SimulatedExecutor {
    /// A fresh executor with no launches recorded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm a one-shot failure: the next `launch` is rejected with `reason`
    /// (bounded to [`REASON_CHARS_MAX`] characters).
    ///
    /// Test hook for device-side rejection of a well-formed plan.
    pub fn fail_next_launch(&mut self, reason: &str) {
        self.fail_next = Some(bound_reason(reason));
    }

    /// Launches accepted so far.
    #[must_use]
    pub fn launches(&self) -> u64 {
        self.launches
    }
}

#[cfg(feature = "simulated")]
impl SimulatedExecutor {
    /// Re-derive the total thread count with checked arithmetic.
    fn checked_total_threads(plan: &LaunchPlan) -> Result<u64, KernelError> {
        let grid = plan.launch().grid();
        let block = plan.launch().block();
        let grid_blocks = u64::from(grid.x)
            .checked_mul(u64::from(grid.y))
            .and_then(|v| v.checked_mul(u64::from(grid.z)))
            .ok_or(KernelError::ArithmeticOverflow {
                what: "simulated grid product",
            })?;
        let block_threads = u64::from(block.x)
            .checked_mul(u64::from(block.y))
            .and_then(|v| v.checked_mul(u64::from(block.z)))
            .ok_or(KernelError::ArithmeticOverflow {
                what: "simulated block product",
            })?;
        grid_blocks
            .checked_mul(block_threads)
            .ok_or(KernelError::ArithmeticOverflow {
                what: "simulated total product",
            })
    }

    /// Deterministic checksum over the kernel name and launch geometry.
    fn plan_checksum(plan: &LaunchPlan) -> u64 {
        let launch = plan.launch();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(launch.kernel().as_str().as_bytes());
        for dim in [
            launch.grid().x,
            launch.grid().y,
            launch.grid().z,
            launch.block().x,
            launch.block().y,
            launch.block().z,
        ] {
            bytes.extend_from_slice(&dim.to_le_bytes());
        }
        bytes.extend_from_slice(&plan.sequence().to_le_bytes());
        fnv1a(&bytes)
    }
}

#[cfg(feature = "simulated")]
impl KernelExecutor for SimulatedExecutor {
    fn launch(&mut self, plan: &LaunchPlan) -> Result<LaunchReceipt, KernelError> {
        if let Some(reason) = self.fail_next.take() {
            return Err(KernelError::ExecutorRejected { reason });
        }
        let total_threads = Self::checked_total_threads(plan)?;
        // The plan type guarantees validation; the re-derivation must agree.
        debug_assert_eq!(total_threads, plan.launch().total_threads());
        self.launches += 1;
        Ok(LaunchReceipt {
            plan_sequence: plan.sequence(),
            kernel: plan.launch().kernel().as_str().to_owned(),
            total_threads,
            checksum: Self::plan_checksum(plan),
        })
    }

    fn synchronize(&mut self) -> Result<(), KernelError> {
        Ok(())
    }
}
