//! Tensor→tile partitions with exact, disjoint coverage.
//!
//! A partition splits a tensor into tiles so that every tensor element
//! belongs to exactly one tile. Exact divisibility is required per axis:
//! partial edge tiles would need bounds checks inside the tile program, and
//! this crate models the discipline where the host guarantees full tiles.

use crate::error::ComputeError;
use crate::tile::{TILE_RANK_MAX, TensorShape, TileShape};

/// Largest number of tiles one partition may describe.
///
/// A pathological 4-D partition could otherwise name 2^40 tiles; this bound
/// keeps `0..tile_count` iteration finite and the grid product inside `u64`.
pub const PARTITION_TILES_MAX: u64 = 1 << 32;

/// A validated partition: tensor shape, tile shape, per-axis grid, and the
/// checked tile count.
///
/// The invariant: tensor and tile share a rank, every tensor extent is an
/// exact multiple of the matching tile extent, and the grid product is
/// `1..=PARTITION_TILES_MAX`. Tiles are therefore disjoint and cover the
/// tensor exactly — the property the ownership layer relies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Partition {
    tensor: TensorShape,
    tile: TileShape,
    grid: [u32; TILE_RANK_MAX],
    tile_count: u64,
}

impl Partition {
    /// Builds the partition of `tensor` into `tile`-shaped tiles.
    ///
    /// # Contract
    /// - Accepts: equal ranks, per-axis exact divisibility, grid product
    ///   within `PARTITION_TILES_MAX`.
    /// - Rejects: rank mismatch, a non-divisible axis, grid overflow, or
    ///   too many tiles — with a typed error; nothing is published.
    pub fn new(tensor: TensorShape, tile: TileShape) -> Result<Self, ComputeError> {
        if tensor.rank() != tile.rank() {
            return Err(ComputeError::RankMismatch {
                tensor_rank: tensor.rank(),
                tile_rank: tile.rank(),
            });
        }
        let mut grid = [0u32; TILE_RANK_MAX];
        let mut tile_count: u64 = 1;
        // `tensor.rank() <= TILE_RANK_MAX`, so `take` visits exactly the
        // live axes; `slot` is the grid cell for `axis`.
        for (axis, slot) in grid.iter_mut().enumerate().take(tensor.rank()) {
            // Ranks are equal and validated, so both extents exist.
            let tensor_extent = tensor.extent(axis).unwrap_or(0);
            let tile_extent = tile.extent(axis).unwrap_or(0);
            debug_assert!(tensor_extent > 0 && tile_extent > 0);
            if !tensor_extent.is_multiple_of(tile_extent) {
                return Err(ComputeError::NotDivisible {
                    axis,
                    tensor_extent,
                    tile_extent,
                });
            }
            let axis_tiles = tensor_extent / tile_extent;
            *slot = axis_tiles;
            tile_count = tile_count
                .checked_mul(u64::from(axis_tiles))
                .ok_or(ComputeError::GridOverflow)?;
        }
        if tile_count > PARTITION_TILES_MAX {
            return Err(ComputeError::TooManyTiles { count: tile_count });
        }
        Ok(Self {
            tensor,
            tile,
            grid,
            tile_count,
        })
    }

    /// The partitioned tensor shape.
    pub fn tensor(&self) -> TensorShape {
        self.tensor
    }

    /// The tile shape every tile shares.
    pub fn tile(&self) -> TileShape {
        self.tile
    }

    /// Tiles per axis, in axis order.
    pub fn grid(&self) -> &[u32] {
        &self.grid[..self.tensor.rank()]
    }

    /// Total tiles: the checked grid product.
    pub fn tile_count(&self) -> u64 {
        self.tile_count
    }

    /// Per-axis origin (element offset) of tile `index`, row-major: the
    /// last axis varies fastest.
    ///
    /// # Contract
    /// - Accepts: `index < tile_count`.
    /// - Rejects: out-of-range indices with a typed error.
    pub fn tile_origin(&self, index: u64) -> Result<[u32; TILE_RANK_MAX], ComputeError> {
        if index >= self.tile_count {
            return Err(ComputeError::TileIndexOutOfRange {
                index,
                tile_count: self.tile_count,
            });
        }
        Ok(self.origin_unchecked(index))
    }

    /// Mixed-radix decomposition without the bounds check. Private: callers
    /// must have established `index < tile_count`.
    fn origin_unchecked(&self, index: u64) -> [u32; TILE_RANK_MAX] {
        let mut origin = [0u32; TILE_RANK_MAX];
        let mut remaining = index;
        for axis in (0..self.tensor.rank()).rev() {
            let axis_tiles = u64::from(self.grid[axis]);
            debug_assert!(axis_tiles >= 1);
            let coord = remaining % axis_tiles;
            remaining /= axis_tiles;
            let tile_extent = u64::from(self.tile.extent(axis).unwrap_or(0));
            // coord < axis_tiles and coord * tile_extent < tensor_extent <= u32::MAX.
            origin[axis] = (coord * tile_extent) as u32;
        }
        debug_assert_eq!(remaining, 0);
        origin
    }

    /// Calls `visit` for every tile origin in index order, stopping early on
    /// `Err`. The count is bounded by [`PARTITION_TILES_MAX`].
    pub fn for_each_tile<E>(
        &self,
        mut visit: impl FnMut(u64, [u32; TILE_RANK_MAX]) -> Result<(), E>,
    ) -> Result<(), E> {
        for index in 0..self.tile_count {
            visit(index, self.origin_unchecked(index))?;
        }
        Ok(())
    }
}
