//! Validated tile and tensor shapes.
//!
//! A shape is a rank plus one extent per axis. Tiles are small (bounded for
//! mapping onto one thread block); tensors are larger (bounded for a sane
//! memory budget). Both are validated at construction, so downstream code can
//! treat the stored extents as already within bounds.

use crate::error::ComputeError;

/// Largest rank (number of axes) a shape may carry.
///
/// Four axes cover the batched-matrix cases without inviting unbounded
/// dimension vectors.
pub const TILE_RANK_MAX: usize = 4;

/// Largest single-axis extent of a tile, in elements.
///
/// 1024 matches the CUDA maximum threads-per-block: the natural upper bound
/// for one tile mapped onto a single thread block.
pub const TILE_EXTENT_MAX: u32 = 1024;

/// Largest total element count of one tile.
///
/// 2^24 f32 elements are 64 MiB of tile scratch: generous for a tile while
/// keeping the checked product far below `u64` overflow.
pub const TILE_ELEMENT_COUNT_MAX: u64 = 1 << 24;

/// Largest single-axis extent of a tensor, in elements.
pub const TENSOR_EXTENT_MAX: u32 = 1 << 24;

/// Largest total element count of one tensor.
///
/// 2^32 f32 elements are 16 GiB: a whole-GPU budget expressed as a shape
/// bound, independent of any allocator.
pub const TENSOR_ELEMENT_COUNT_MAX: u64 = 1 << 32;

/// Shared validation core: rank, per-axis extents, and the checked element
/// product. Private so the two public shapes cannot drift apart.
fn checked_extents(
    extents: &[u32],
    extent_max: u32,
    count_max: u64,
) -> Result<([u32; TILE_RANK_MAX], usize, u64), ComputeError> {
    if extents.is_empty() || extents.len() > TILE_RANK_MAX {
        return Err(ComputeError::InvalidRank {
            rank: extents.len(),
        });
    }
    let mut stored = [0u32; TILE_RANK_MAX];
    let mut product: u64 = 1;
    for (axis, extent) in extents.iter().enumerate() {
        if *extent == 0 {
            return Err(ComputeError::ZeroExtent { axis });
        }
        if *extent > extent_max {
            return Err(ComputeError::ExtentTooLarge {
                axis,
                extent: *extent,
            });
        }
        stored[axis] = *extent;
        product = product
            .checked_mul(u64::from(*extent))
            .ok_or(ComputeError::ElementCountOverflow)?;
        if product > count_max {
            return Err(ComputeError::ElementCountTooLarge { count: product });
        }
    }
    Ok((stored, extents.len(), product))
}

/// A validated tile shape: the unit of work one tile program processes.
///
/// The invariant is `1 <= rank <= TILE_RANK_MAX`, every extent in
/// `1..=TILE_EXTENT_MAX`, and the element product in
/// `1..=TILE_ELEMENT_COUNT_MAX`. Only [`TileShape::new`] constructs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileShape {
    rank: usize,
    extents: [u32; TILE_RANK_MAX],
    element_count: u64,
}

impl TileShape {
    /// Builds a tile shape from per-axis extents.
    ///
    /// # Contract
    /// - Accepts: 1..=TILE_RANK_MAX extents, each `1..=TILE_EXTENT_MAX`,
    ///   with a checked product of `1..=TILE_ELEMENT_COUNT_MAX`.
    /// - Rejects: empty input, too many axes, zero or oversized extents, or
    ///   an over-large product — with a typed error and no heap allocation.
    pub fn new(extents: &[u32]) -> Result<Self, ComputeError> {
        let (stored, rank, element_count) =
            checked_extents(extents, TILE_EXTENT_MAX, TILE_ELEMENT_COUNT_MAX)?;
        Ok(Self {
            rank,
            extents: stored,
            element_count,
        })
    }

    /// Axes in this shape: `1..=TILE_RANK_MAX`.
    pub fn rank(&self) -> usize {
        self.rank
    }

    /// Extent of one axis, in elements. `None` when `axis >= rank`.
    pub fn extent(&self, axis: usize) -> Option<u32> {
        if axis < self.rank {
            Some(self.extents[axis])
        } else {
            None
        }
    }

    /// All extents, in axis order.
    pub fn extents(&self) -> &[u32] {
        &self.extents[..self.rank]
    }

    /// Total elements: the checked product of the extents.
    pub fn element_count(&self) -> u64 {
        self.element_count
    }
}

/// A validated tensor shape: the whole object a launch reads or writes.
///
/// Same structural rules as [`TileShape`] with the looser tensor bounds
/// ([`TENSOR_EXTENT_MAX`], [`TENSOR_ELEMENT_COUNT_MAX`]). A distinct type so
/// a tile can never be silently used where a tensor is expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TensorShape {
    rank: usize,
    extents: [u32; TILE_RANK_MAX],
    element_count: u64,
}

impl TensorShape {
    /// Builds a tensor shape from per-axis extents.
    ///
    /// # Contract
    /// - Accepts: 1..=TILE_RANK_MAX extents, each `1..=TENSOR_EXTENT_MAX`,
    ///   with a checked product of `1..=TENSOR_ELEMENT_COUNT_MAX`.
    /// - Rejects: empty input, too many axes, zero or oversized extents, or
    ///   an over-large product — with a typed error and no heap allocation.
    pub fn new(extents: &[u32]) -> Result<Self, ComputeError> {
        let (stored, rank, element_count) =
            checked_extents(extents, TENSOR_EXTENT_MAX, TENSOR_ELEMENT_COUNT_MAX)?;
        Ok(Self {
            rank,
            extents: stored,
            element_count,
        })
    }

    /// Axes in this shape: `1..=TILE_RANK_MAX`.
    pub fn rank(&self) -> usize {
        self.rank
    }

    /// Extent of one axis, in elements. `None` when `axis >= rank`.
    pub fn extent(&self, axis: usize) -> Option<u32> {
        if axis < self.rank {
            Some(self.extents[axis])
        } else {
            None
        }
    }

    /// All extents, in axis order.
    pub fn extents(&self) -> &[u32] {
        &self.extents[..self.rank]
    }

    /// Total elements: the checked product of the extents.
    pub fn element_count(&self) -> u64 {
        self.element_count
    }
}
