//! Launch configuration and its validation.
//!
//! Adapted from Mojo's GPU launch model: a kernel is enqueued with a grid of
//! thread blocks (`grid_dim`) and threads per block (`block_dim`), each
//! one-to-three dimensional. Mojo itself type-checks kernel arguments at
//! compile time but leaves dimension sanity to the device driver (an
//! out-of-range `block_dim` surfaces as a raw driver error). This module does
//! the host-side checking the driver does not: every dimension is range
//! checked, every product uses checked arithmetic, and a launch that cannot
//! satisfy the device limits is rejected with a typed error before any plan
//! is committed.
//!
//! [`ValidatedLaunch`] is only constructible through
//! [`LaunchConfig::validate`]; downstream code (planner, executor) takes a
//! `ValidatedLaunch`, so an unvalidated launch is unrepresentable.

use crate::descriptor::KernelRegistry;
use crate::error::{DimAxis, KernelError};

/// Maximum threads per block on the generic device profile.
///
/// Verified against real hardware behavior: NVIDIA GPUs reject more than
/// 1024 threads per block at the driver level (see docs/decisions.md).
pub const THREADS_PER_BLOCK_MAX: u32 = 1024;
/// Maximum shared memory per block on the generic device profile (48 KiB).
pub const SHARED_MEMORY_BYTES_PER_BLOCK_MAX: u32 = 48 * 1024;
/// Maximum value of any single grid or block dimension.
pub const GRID_DIM_MAX: u32 = 65_535;
/// Maximum total threads (grid blocks x block threads) per launch.
///
/// A conservative budget, not a hardware ceiling: it keeps the host-side
/// work estimate honest before a plan is committed.
pub const TOTAL_THREADS_PER_LAUNCH_MAX: u64 = 1 << 31;

/// One-to-three dimensional extent, mirroring Mojo's `grid_dim`/`block_dim`
/// arguments (each accepts an int or an (x, y, z) tuple).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dim3 {
    /// Extent on the x axis.
    pub x: u32,
    /// Extent on the y axis.
    pub y: u32,
    /// Extent on the z axis.
    pub z: u32,
}

impl Dim3 {
    /// Build a 3D extent. Plain data; range checks happen at validation.
    #[must_use]
    pub const fn new(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }

    /// A 1D extent of `n` (y and z are 1).
    #[must_use]
    pub const fn one_d(n: u32) -> Self {
        Self { x: n, y: 1, z: 1 }
    }

    /// A 2D extent (z is 1).
    #[must_use]
    pub const fn two_d(x: u32, y: u32) -> Self {
        Self { x, y, z: 1 }
    }

    /// Checked element product as u64. `None` on overflow.
    fn checked_product(self) -> Option<u64> {
        let xy = u64::from(self.x).checked_mul(u64::from(self.y))?;
        xy.checked_mul(u64::from(self.z))
    }

    /// Range-check every axis: non-zero and within `dim_max`.
    fn validate_axes(self, dim_max: u32) -> Result<(), KernelError> {
        for (axis, value) in [
            (DimAxis::X, self.x),
            (DimAxis::Y, self.y),
            (DimAxis::Z, self.z),
        ] {
            if value == 0 {
                return Err(KernelError::ZeroDim { axis });
            }
            if value > dim_max {
                return Err(KernelError::DimTooLarge {
                    axis,
                    value,
                    max: dim_max,
                });
            }
        }
        Ok(())
    }
}

/// Device limits a launch is validated against.
///
/// The [`DeviceLimits::generic`] profile models widely supported hardware
/// (1024 threads/block, 48 KiB shared memory/block). A real device profile
/// would be populated from device queries; the shape stays the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceLimits {
    /// Maximum threads in one block.
    pub threads_per_block_max: u32,
    /// Maximum shared-memory bytes in one block.
    pub shared_memory_bytes_per_block_max: u32,
    /// Maximum total threads per launch.
    pub total_threads_per_launch_max: u64,
    /// Maximum value of any single grid/block dimension.
    pub grid_dim_max: u32,
}

impl DeviceLimits {
    /// The generic device profile.
    #[must_use]
    pub const fn generic() -> Self {
        Self {
            threads_per_block_max: THREADS_PER_BLOCK_MAX,
            shared_memory_bytes_per_block_max: SHARED_MEMORY_BYTES_PER_BLOCK_MAX,
            total_threads_per_launch_max: TOTAL_THREADS_PER_LAUNCH_MAX,
            grid_dim_max: GRID_DIM_MAX,
        }
    }
}

/// An unvalidated kernel launch request.
///
/// Analogous to the arguments of Mojo's `enqueue_function`: the kernel's
/// name, `grid_dim`, `block_dim`, plus the resource claims the host must
/// check (shared memory, argument count, payload bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchConfig {
    kernel: crate::descriptor::KernelName,
    grid: Dim3,
    block: Dim3,
    shared_memory_bytes: u32,
    arg_count: u8,
    payload_bytes: u32,
}

impl LaunchConfig {
    /// Build a launch request. Plain data; call [`Self::validate`] to check
    /// it against a registry and device limits.
    #[must_use]
    pub fn new(
        kernel: crate::descriptor::KernelName,
        grid: Dim3,
        block: Dim3,
        shared_memory_bytes: u32,
        arg_count: u8,
        payload_bytes: u32,
    ) -> Self {
        Self {
            kernel,
            grid,
            block,
            shared_memory_bytes,
            arg_count,
            payload_bytes,
        }
    }

    /// Validate the launch: registry lookup, dimension ranges, checked
    /// thread products, shared-memory and payload budgets.
    ///
    /// Checks run cheapest-first (name lookup, axis ranges) so the common
    /// rejections never compute a product. Rejection leaves the registry
    /// and every other state untouched.
    ///
    /// # Errors
    ///
    /// Any [`KernelError`] variant describing the first failed check.
    pub fn validate(
        &self,
        registry: &KernelRegistry,
        limits: &DeviceLimits,
    ) -> Result<ValidatedLaunch, KernelError> {
        let descriptor = registry
            .get(&self.kernel)
            .ok_or_else(|| KernelError::UnknownKernel {
                name: self.kernel.as_str().to_owned(),
            })?;

        self.grid.validate_axes(limits.grid_dim_max)?;
        self.block.validate_axes(limits.grid_dim_max)?;

        let block_threads =
            self.block
                .checked_product()
                .ok_or(KernelError::ArithmeticOverflow {
                    what: "block thread product",
                })?;
        if block_threads > u64::from(limits.threads_per_block_max) {
            return Err(KernelError::ThreadsPerBlockExceeded {
                requested: block_threads,
                limit: limits.threads_per_block_max,
            });
        }

        let grid_blocks = self
            .grid
            .checked_product()
            .ok_or(KernelError::ArithmeticOverflow {
                what: "grid block product",
            })?;
        let total_threads =
            grid_blocks
                .checked_mul(block_threads)
                .ok_or(KernelError::ArithmeticOverflow {
                    what: "total thread product",
                })?;
        if total_threads > limits.total_threads_per_launch_max {
            return Err(KernelError::TotalThreadsExceeded {
                requested: total_threads,
                limit: limits.total_threads_per_launch_max,
            });
        }

        if self.shared_memory_bytes > limits.shared_memory_bytes_per_block_max {
            return Err(KernelError::SharedMemoryExceeded {
                requested: self.shared_memory_bytes,
                limit: limits.shared_memory_bytes_per_block_max,
            });
        }
        if self.shared_memory_bytes < descriptor.shared_memory_bytes() {
            return Err(KernelError::SharedMemoryInsufficient {
                required: descriptor.shared_memory_bytes(),
                provided: self.shared_memory_bytes,
            });
        }

        if self.arg_count > descriptor.arg_count_max() {
            return Err(KernelError::ArgCountExceeded {
                requested: self.arg_count,
                max: descriptor.arg_count_max(),
            });
        }
        let payload_limit = descriptor.payload_bytes_max();
        if self.payload_bytes > payload_limit {
            return Err(KernelError::PayloadBytesExceeded {
                requested: self.payload_bytes,
                limit: payload_limit,
            });
        }

        Ok(ValidatedLaunch {
            kernel: self.kernel.clone(),
            version: descriptor.version(),
            grid: self.grid,
            block: self.block,
            shared_memory_bytes: self.shared_memory_bytes,
            arg_count: self.arg_count,
            payload_bytes: self.payload_bytes,
            total_threads,
        })
    }
}

/// A launch that passed every host-side check.
///
/// Only [`LaunchConfig::validate`] constructs this. The planner and the
/// executor accept `ValidatedLaunch`, so unvalidated launches cannot reach
/// device planning code. All fields are immutable snapshots of the validated
/// request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedLaunch {
    kernel: crate::descriptor::KernelName,
    version: crate::descriptor::KernelVersion,
    grid: Dim3,
    block: Dim3,
    shared_memory_bytes: u32,
    arg_count: u8,
    payload_bytes: u32,
    total_threads: u64,
}

impl ValidatedLaunch {
    /// Kernel name this launch targets.
    #[must_use]
    pub fn kernel(&self) -> &crate::descriptor::KernelName {
        &self.kernel
    }

    /// Kernel version the launch was validated against.
    #[must_use]
    pub fn version(&self) -> crate::descriptor::KernelVersion {
        self.version
    }

    /// Grid dimensions (thread blocks).
    #[must_use]
    pub fn grid(&self) -> Dim3 {
        self.grid
    }

    /// Block dimensions (threads per block).
    #[must_use]
    pub fn block(&self) -> Dim3 {
        self.block
    }

    /// Shared-memory bytes per block.
    #[must_use]
    pub fn shared_memory_bytes(&self) -> u32 {
        self.shared_memory_bytes
    }

    /// Argument count the launch passes.
    #[must_use]
    pub fn arg_count(&self) -> u8 {
        self.arg_count
    }

    /// Payload bytes the launch carries.
    #[must_use]
    pub fn payload_bytes(&self) -> u32 {
        self.payload_bytes
    }

    /// Total threads: grid blocks x block threads, checked at validation.
    #[must_use]
    pub fn total_threads(&self) -> u64 {
        self.total_threads
    }
}
