//! Launch configuration: grid, block, and shared memory, validated.
//!
//! A launch configuration answers "how many threads, arranged how, with how
//! much scratch memory". The CUDA limits are enforced here — before any
//! backend sees the config — so an invalid launch is a typed error, never a
//! driver fault.

use crate::error::CudaError;

/// Most threads in one thread block: the CUDA hardware limit.
pub const BLOCK_THREADS_MAX: u32 = 1024;

/// Most dynamic shared memory per launch, in bytes: 48 KiB, the default
/// limit without special opt-in.
pub const SHARED_MEM_BYTES_MAX: u32 = 48 * 1024;

/// Largest single grid axis: `2^31 - 1`, the CUDA limit for the x dimension
/// (y and z are smaller in hardware, but this single bound is the
/// conservative, documented simplification).
pub const GRID_DIM_AXIS_MAX: u32 = 2_147_483_647;

/// One launch dimension: grid or block axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// x axis.
    X,
    /// y axis.
    Y,
    /// z axis.
    Z,
}

/// Three launch dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dim3 {
    /// x extent.
    pub x: u32,
    /// y extent.
    pub y: u32,
    /// z extent.
    pub z: u32,
}

impl Dim3 {
    /// Builds a dimension triple. Values are checked by
    /// [`LaunchConfig::validate`], not here.
    pub fn new(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }

    /// Axis value by axis.
    fn get(&self, axis: Axis) -> u32 {
        match axis {
            Axis::X => self.x,
            Axis::Y => self.y,
            Axis::Z => self.z,
        }
    }
}

/// How to launch a kernel: grid of blocks, threads per block, dynamic
/// shared memory.
///
/// Fields are public for inspection, but a backend must only consume a
/// [`ValidatedLaunch`]; use [`LaunchConfig::validate`] to get one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchConfig {
    /// Blocks per grid axis.
    pub grid: Dim3,
    /// Threads per block axis.
    pub block: Dim3,
    /// Dynamic shared memory per block, in bytes.
    pub shared_mem_bytes: u32,
}

/// A launch configuration that passed every limit check, with the derived
/// thread counts computed under checked arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedLaunch {
    /// The validated configuration.
    pub config: LaunchConfig,
    /// Threads per block: `block.x * block.y * block.z`, `1..=1024`.
    pub block_threads: u32,
    /// Total threads: `grid.x * grid.y * grid.z * block_threads`.
    pub total_threads: u64,
}

impl LaunchConfig {
    /// Validates the configuration against the CUDA limits.
    ///
    /// # Contract
    /// - Accepts: every grid and block axis `>= 1`, grid axes
    ///   `<= GRID_DIM_AXIS_MAX`, block thread product `<= 1024`,
    ///   `shared_mem_bytes <= SHARED_MEM_BYTES_MAX`.
    /// - Rejects: zero axes, oversized grid axes, over-full blocks (with the
    ///   checked product, never a wrapping multiply), or excess shared
    ///   memory — with a typed error.
    pub fn validate(&self) -> Result<ValidatedLaunch, CudaError> {
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            let grid_value = self.grid.get(axis);
            if grid_value == 0 {
                return Err(CudaError::ZeroDim {
                    what: axis_grid_name(axis),
                });
            }
            if grid_value > GRID_DIM_AXIS_MAX {
                return Err(CudaError::GridDimTooLarge {
                    what: axis_grid_name(axis),
                    value: grid_value,
                });
            }
            if self.block.get(axis) == 0 {
                return Err(CudaError::ZeroDim {
                    what: axis_block_name(axis),
                });
            }
        }
        let block_threads = u64::from(self.block.x)
            .checked_mul(u64::from(self.block.y))
            .and_then(|partial| partial.checked_mul(u64::from(self.block.z)))
            .ok_or(CudaError::BlockThreadsOverflow)?;
        if block_threads > u64::from(BLOCK_THREADS_MAX) {
            return Err(CudaError::BlockThreadsTooMany {
                threads: block_threads,
            });
        }
        // block_threads >= 1 (axes >= 1) and <= 1024: the cast is exact.
        let block_threads = block_threads as u32;
        if self.shared_mem_bytes > SHARED_MEM_BYTES_MAX {
            return Err(CudaError::SharedMemTooLarge {
                bytes: self.shared_mem_bytes,
            });
        }
        let grid_blocks = u64::from(self.grid.x)
            .checked_mul(u64::from(self.grid.y))
            .and_then(|partial| partial.checked_mul(u64::from(self.grid.z)))
            .ok_or(CudaError::BlockThreadsOverflow)?;
        let total_threads = grid_blocks
            .checked_mul(u64::from(block_threads))
            .ok_or(CudaError::BlockThreadsOverflow)?;
        Ok(ValidatedLaunch {
            config: *self,
            block_threads,
            total_threads,
        })
    }
}

/// Error-context label for a grid axis.
fn axis_grid_name(axis: Axis) -> &'static str {
    match axis {
        Axis::X => "grid.x",
        Axis::Y => "grid.y",
        Axis::Z => "grid.z",
    }
}

/// Error-context label for a block axis.
fn axis_block_name(axis: Axis) -> &'static str {
    match axis {
        Axis::X => "block.x",
        Axis::Y => "block.y",
        Axis::Z => "block.z",
    }
}
