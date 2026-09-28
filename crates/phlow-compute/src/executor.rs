//! Deterministic tile executor: the CPU stand-in for a tile runtime.
//!
//! The executor runs one validated launch against a caller-supplied per-tile
//! behavior, visiting tiles in index order. It performs no GPU work; its
//! value is that the ownership checks a device backend would enforce —
//! disjoint tiles, exact coverage, deterministic order — run here first,
//! where failures are cheap and observable.

use crate::error::ComputeError;
use crate::ownership::{LaunchArgs, TileView};

/// Outcome of one executed launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionReport {
    /// Tiles the partition described.
    pub tiles_total: u64,
    /// Tiles whose behavior returned `Ok`.
    pub tiles_completed: u64,
}

/// Runs tile programs on the CPU in deterministic tile-index order.
///
/// The type carries no state; `execute` is an associated function so there
/// is no executor instance to own, configure, or shut down.
pub struct TileExecutor;

impl TileExecutor {
    /// Executes `per_tile` for every tile of the launch's exclusive output,
    /// in index order `0..tile_count`.
    ///
    /// # Contract
    /// - Accepts: validated [`LaunchArgs`]; the behavior may read the
    ///   launch's shared inputs through its own handle mapping.
    /// - The behavior receives each [`TileView`] exactly once, in order.
    /// - The first failing tile aborts the launch with
    ///   [`ComputeError::TileFault`] naming the tile index; tiles after it
    ///   never run, and no partial report is returned.
    /// - Returns [`ExecutionReport`] with `tiles_completed == tiles_total`
    ///   on success.
    ///
    /// The tile count is bounded by [`crate::PARTITION_TILES_MAX`], so the
    /// loop is finite.
    pub fn execute(
        args: &LaunchArgs,
        mut per_tile: impl FnMut(TileView) -> Result<(), &'static str>,
    ) -> Result<ExecutionReport, ComputeError> {
        let tiles_total = args.output().partition().tile_count();
        let mut tiles_completed: u64 = 0;
        for tile in args.output().tiles() {
            if let Err(reason) = per_tile(tile) {
                return Err(ComputeError::TileFault {
                    index: tile.index,
                    reason,
                });
            }
            tiles_completed += 1;
        }
        debug_assert_eq!(tiles_completed, tiles_total);
        Ok(ExecutionReport {
            tiles_total,
            tiles_completed,
        })
    }
}
