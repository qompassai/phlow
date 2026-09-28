//! Tile-block coordinates: the CPU model of `program_id(axis)`.
//!
//! In the tile model each tile program runs the entry function once and
//! processes one output tile. Programs query their per-axis grid coordinates;
//! this module provides the same mapping on the host so tests and the
//! simulator can reason about which tile a program index names.

use crate::error::ComputeError;

/// Grid axes exposed to tile programs: x, y, z.
pub const PROGRAM_AXES: usize = 3;

/// Coordinates of one tile program inside the grid: the host-side model of
/// cutile-rs `program_id(axis)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramId {
    coords: [u32; PROGRAM_AXES],
}

impl ProgramId {
    /// Per-axis coordinates, x first.
    pub fn coords(&self) -> [u32; PROGRAM_AXES] {
        self.coords
    }

    /// The coordinate on one axis. `None` when `axis >= PROGRAM_AXES`.
    pub fn axis(&self, axis: usize) -> Option<u32> {
        self.coords.get(axis).copied()
    }
}

/// Grid geometry: tile counts per axis, row-major with x fastest.
///
/// A partition of rank 1..=3 maps directly; higher ranks have no 3-D grid
/// counterpart and are rejected rather than silently flattened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridGeometry {
    extents: [u32; PROGRAM_AXES],
    axes: usize,
    programs: u64,
}

impl GridGeometry {
    /// Builds grid geometry from per-axis tile counts.
    ///
    /// # Contract
    /// - Accepts: 1..=PROGRAM_AXES counts, each `>= 1`, with a checked
    ///   product inside `u64`.
    /// - Rejects: empty input, too many axes, or a zero axis — with a typed
    ///   error.
    pub fn new(tile_counts: &[u32]) -> Result<Self, ComputeError> {
        if tile_counts.is_empty() || tile_counts.len() > PROGRAM_AXES {
            return Err(ComputeError::InvalidGridRank {
                rank: tile_counts.len(),
            });
        }
        let mut extents = [1u32; PROGRAM_AXES];
        let mut programs: u64 = 1;
        for (axis, count) in tile_counts.iter().enumerate() {
            if *count == 0 {
                return Err(ComputeError::ZeroExtent { axis });
            }
            extents[axis] = *count;
            programs = programs
                .checked_mul(u64::from(*count))
                .ok_or(ComputeError::GridOverflow)?;
        }
        Ok(Self {
            extents,
            axes: tile_counts.len(),
            programs,
        })
    }

    /// Tile programs on one axis: the model of `num_programs(axis)`.
    pub fn num_programs(&self, axis: usize) -> Option<u32> {
        if axis < self.axes {
            Some(self.extents[axis])
        } else {
            None
        }
    }

    /// Total tile programs in the grid.
    pub fn programs(&self) -> u64 {
        self.programs
    }

    /// Coordinates of the `index`-th tile program, row-major.
    ///
    /// # Contract
    /// - Accepts: `index < programs`.
    /// - Rejects: out-of-range indices with a typed error.
    pub fn program_id(&self, index: u64) -> Result<ProgramId, ComputeError> {
        if index >= self.programs {
            return Err(ComputeError::ProgramIndexOutOfRange {
                index,
                programs: self.programs,
            });
        }
        let mut coords = [0u32; PROGRAM_AXES];
        let mut remaining = index;
        for axis in (0..self.axes).rev() {
            let extent = u64::from(self.extents[axis]);
            coords[axis] = (remaining % extent) as u32;
            remaining /= extent;
        }
        debug_assert_eq!(remaining, 0);
        Ok(ProgramId { coords })
    }

    /// Inverse of [`GridGeometry::program_id`]: the linear index of
    /// `coords`. Callers must pass coordinates inside the grid; out-of-range
    /// coordinates are rejected rather than wrapped.
    pub fn linear_index(&self, coords: [u32; PROGRAM_AXES]) -> Result<u64, ComputeError> {
        let mut index: u64 = 0;
        for axis in 0..self.axes {
            if coords[axis] >= self.extents[axis] {
                return Err(ComputeError::ProgramIndexOutOfRange {
                    index: u64::from(coords[axis]),
                    programs: u64::from(self.extents[axis]),
                });
            }
            index = index
                .checked_mul(u64::from(self.extents[axis]))
                .and_then(|base| base.checked_add(u64::from(coords[axis])))
                .ok_or(ComputeError::GridOverflow)?;
        }
        Ok(index)
    }
}
