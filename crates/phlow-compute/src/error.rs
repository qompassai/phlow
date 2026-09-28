//! Typed errors for tile-shape, partition, ownership, and execution failures.
//!
//! Every variant carries bounded context: indices and counts, never tensor
//! contents or unbounded strings.

use std::fmt;

/// Failures from validating shapes, partitions, launch arguments, and tile
/// execution.
///
/// The contract: malformed input produces one of these variants; a correct
/// implementation never triggers one after validation succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputeError {
    /// A shape carried zero axes, or more than [`crate::TILE_RANK_MAX`].
    InvalidRank {
        /// The rejected axis count.
        rank: usize,
    },
    /// A shape axis had a zero extent.
    ZeroExtent {
        /// The rejected axis.
        axis: usize,
    },
    /// A shape axis exceeded the per-axis bound.
    ExtentTooLarge {
        /// The rejected axis.
        axis: usize,
        /// The rejected extent, in elements.
        extent: u32,
    },
    /// The checked element product exceeded the total-element bound.
    ElementCountTooLarge {
        /// The rejected product, in elements.
        count: u64,
    },
    /// The element product overflowed `u64` before the bound was reached.
    ///
    /// Unreachable with the current public bounds (they are checked first),
    /// kept so the arithmetic stays total if bounds ever change.
    ElementCountOverflow,
    /// A tensor and its tile disagreed on rank.
    RankMismatch {
        /// Tensor axes.
        tensor_rank: usize,
        /// Tile axes.
        tile_rank: usize,
    },
    /// A tensor extent was not an exact multiple of the tile extent.
    NotDivisible {
        /// The offending axis.
        axis: usize,
        /// Tensor extent, in elements.
        tensor_extent: u32,
        /// Tile extent, in elements.
        tile_extent: u32,
    },
    /// The per-axis grid product overflowed `u64`.
    GridOverflow,
    /// A partition would describe more than [`crate::PARTITION_TILES_MAX`]
    /// tiles.
    TooManyTiles {
        /// The rejected tile count.
        count: u64,
    },
    /// A tile index was outside `0..tile_count`.
    TileIndexOutOfRange {
        /// The rejected index.
        index: u64,
        /// Tiles in the partition.
        tile_count: u64,
    },
    /// A launch bound zero read-only inputs.
    NoInputs,
    /// A launch bound more than [`crate::INPUT_COUNT_MAX`] inputs.
    TooManyInputs {
        /// The rejected input count.
        count: usize,
    },
    /// An input handle aliased the exclusive output handle.
    HandleAlias {
        /// The aliased handle.
        handle: u64,
    },
    /// Two inputs shared one handle.
    ///
    /// Read/read sharing is legal, so this is a caller-confusion signal, not
    /// a safety violation; it is still rejected to keep launches explicit.
    DuplicateInputHandle {
        /// The duplicated handle.
        handle: u64,
    },
    /// A per-tile behavior failed during execution.
    TileFault {
        /// Index of the tile whose behavior failed.
        index: u64,
        /// Caller-supplied reason; a static string, never tensor data.
        reason: &'static str,
    },
    /// A grid geometry carried zero axes or more than [`crate::PROGRAM_AXES`].
    InvalidGridRank {
        /// The rejected axis count.
        rank: usize,
    },
    /// A linear tile index was outside the grid.
    ProgramIndexOutOfRange {
        /// The rejected index.
        index: u64,
        /// Tile programs in the grid.
        programs: u64,
    },
}

impl fmt::Display for ComputeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRank { rank } => {
                write!(formatter, "invalid shape rank {rank}")
            }
            Self::ZeroExtent { axis } => {
                write!(formatter, "zero extent on axis {axis}")
            }
            Self::ExtentTooLarge { axis, extent } => {
                write!(
                    formatter,
                    "extent {extent} on axis {axis} exceeds the per-axis bound"
                )
            }
            Self::ElementCountTooLarge { count } => {
                write!(formatter, "element count {count} exceeds the shape bound")
            }
            Self::ElementCountOverflow => {
                write!(formatter, "element count overflowed u64")
            }
            Self::RankMismatch {
                tensor_rank,
                tile_rank,
            } => write!(
                formatter,
                "tensor rank {tensor_rank} does not match tile rank {tile_rank}"
            ),
            Self::NotDivisible {
                axis,
                tensor_extent,
                tile_extent,
            } => write!(
                formatter,
                "tensor extent {tensor_extent} on axis {axis} is not divisible by tile extent {tile_extent}"
            ),
            Self::GridOverflow => {
                write!(formatter, "per-axis grid product overflowed u64")
            }
            Self::TooManyTiles { count } => {
                write!(formatter, "tile count {count} exceeds the partition bound")
            }
            Self::TileIndexOutOfRange { index, tile_count } => {
                write!(formatter, "tile index {index} is outside 0..{tile_count}")
            }
            Self::NoInputs => {
                write!(formatter, "a launch needs at least one read-only input")
            }
            Self::TooManyInputs { count } => {
                write!(
                    formatter,
                    "input count {count} exceeds the per-launch bound"
                )
            }
            Self::HandleAlias { handle } => {
                write!(
                    formatter,
                    "input handle {handle} aliases the exclusive output"
                )
            }
            Self::DuplicateInputHandle { handle } => {
                write!(formatter, "input handle {handle} is bound twice")
            }
            Self::TileFault { index, reason } => {
                write!(formatter, "tile {index} failed: {reason}")
            }
            Self::InvalidGridRank { rank } => {
                write!(formatter, "invalid grid rank {rank}")
            }
            Self::ProgramIndexOutOfRange { index, programs } => {
                write!(formatter, "program index {index} is outside 0..{programs}")
            }
        }
    }
}

impl std::error::Error for ComputeError {}
