//! Tile-based compute abstractions for phlow's GPU lanes.
//!
//! Plain words: a GPU kernel in the tile model does one job — process one
//! tile-shaped region of an output tensor. This crate provides the CPU-side
//! vocabulary for that model: validated tile shapes, tensor partitions whose
//! tiles are disjoint and cover the tensor exactly, an ownership discipline
//! for launch arguments (one exclusive mutable output, shared read-only
//! inputs), tile-block coordinates, and a deterministic executor that runs a
//! per-tile behavior in tile-index order.
//!
//! Concept provenance: the tile-program model, output partitioning into
//! disjoint pieces, and the exclusive-output / shared-input ownership split
//! are adapted from NVlabs `cutile-rs`
//! (https://github.com/nvlabs/cutile-rs, Apache-2.0). Every line of code and
//! prose here is original; no upstream source was copied.
//!
//! # Bounds
//!
//! - [`TILE_RANK_MAX`]: most axes a shape may carry.
//! - [`TILE_EXTENT_MAX`]: most elements on one tile axis.
//! - [`TILE_ELEMENT_COUNT_MAX`]: most elements in one tile.
//! - [`TENSOR_EXTENT_MAX`]: most elements on one tensor axis.
//! - [`TENSOR_ELEMENT_COUNT_MAX`]: most elements in one tensor.
//! - [`PARTITION_TILES_MAX`]: most tiles one partition may describe.
//! - [`INPUT_COUNT_MAX`]: most read-only inputs per launch.
//!
//! # Unsafe policy
//!
//! This crate forbids unsafe code outright.

#![forbid(unsafe_code)]

mod error;
mod executor;
mod ownership;
mod partition;
mod program;
mod tile;

pub use error::ComputeError;
pub use executor::{ExecutionReport, TileExecutor};
pub use ownership::{ExclusiveOutput, INPUT_COUNT_MAX, LaunchArgs, SharedInput, TileView};
pub use partition::{PARTITION_TILES_MAX, Partition};
pub use program::{GridGeometry, PROGRAM_AXES, ProgramId};
pub use tile::{
    TENSOR_ELEMENT_COUNT_MAX, TENSOR_EXTENT_MAX, TILE_ELEMENT_COUNT_MAX, TILE_EXTENT_MAX,
    TILE_RANK_MAX, TensorShape, TileShape,
};
