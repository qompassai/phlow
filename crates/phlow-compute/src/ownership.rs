//! Launch-argument ownership discipline.
//!
//! The rule, adapted from cutile-rs: a launch has exactly one exclusive
//! mutable output and any number of shared read-only inputs. The output is
//! pre-partitioned into disjoint tiles, so tile programs writing through it
//! cannot race each other; inputs are shared because concurrent reads are
//! safe. An input may never alias the output handle — that would be a
//! read/write race across tiles.

use crate::error::ComputeError;
use crate::partition::Partition;
use crate::tile::TILE_RANK_MAX;

/// Largest number of read-only inputs one launch may bind.
///
/// Eight covers multi-operand kernels (e.g. fused attention pieces) while
/// keeping argument validation a small fixed loop.
pub const INPUT_COUNT_MAX: usize = 8;

/// A read-only tensor input shared across every tile program.
///
/// `Copy` like a borrowed handle: sharing is always safe because tile code
/// may only read through it. The handle is opaque here; the backend maps it
/// to a real allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SharedInput {
    handle: u64,
}

impl SharedInput {
    /// Wraps a backend tensor handle as a shared read-only input.
    pub fn new(handle: u64) -> Self {
        Self { handle }
    }

    /// The opaque backend handle.
    pub fn handle(&self) -> u64 {
        self.handle
    }
}

/// The exclusive mutable output of a launch, pre-partitioned into disjoint
/// tiles.
///
/// Exactly one exists per launch — the type-level version of "the mutable
/// output is always the first parameter". Tile programs receive disjoint
/// tile views of it, so concurrent writes cannot overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExclusiveOutput {
    handle: u64,
    partition: Partition,
}

impl ExclusiveOutput {
    /// Wraps a backend tensor handle with its tile partition.
    pub fn new(handle: u64, partition: Partition) -> Self {
        Self { handle, partition }
    }

    /// The opaque backend handle.
    pub fn handle(&self) -> u64 {
        self.handle
    }

    /// The partition describing this output's tiles.
    pub fn partition(&self) -> Partition {
        self.partition
    }

    /// Every tile view in deterministic index order: `0..tile_count`.
    pub fn tiles(&self) -> impl Iterator<Item = TileView> {
        let partition = self.partition;
        (0..partition.tile_count()).map(move |index| TileView {
            index,
            origin: partition
                .tile_origin(index)
                .unwrap_or([0u32; TILE_RANK_MAX]),
            rank: partition.tensor().rank(),
        })
    }
}

/// One tile's view of the exclusive output: its index and per-axis origin.
///
/// The view borrows nothing; the backend resolves `index` to the tile's
/// device memory. Origins are computed from a validated partition, so the
/// `unwrap_or` fallback is unreachable — it exists only because iterators
/// cannot return `Result` from `next`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileView {
    /// Tile index in `0..tile_count`, row-major.
    pub index: u64,
    /// Per-axis element offset of the tile origin.
    pub origin: [u32; TILE_RANK_MAX],
    /// Axes carried by the partition.
    pub rank: usize,
}

/// Validated launch arguments: one exclusive output plus shared inputs.
///
/// The invariant: at least one and at most [`INPUT_COUNT_MAX`] inputs, no
/// input aliases the output handle, and no input handle repeats. Rejected
/// validation leaves nothing published — construction is all-or-nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchArgs {
    output: ExclusiveOutput,
    inputs: [Option<SharedInput>; INPUT_COUNT_MAX],
    input_count: usize,
}

impl LaunchArgs {
    /// Bundles the exclusive output with its read-only inputs.
    ///
    /// # Contract
    /// - Accepts: 1..=INPUT_COUNT_MAX inputs, none aliasing the output
    ///   handle, no duplicated input handles.
    /// - Rejects: empty input lists, too many inputs, output aliasing, or
    ///   duplicated handles — with a typed error.
    pub fn new(output: ExclusiveOutput, inputs: &[SharedInput]) -> Result<Self, ComputeError> {
        if inputs.is_empty() {
            return Err(ComputeError::NoInputs);
        }
        if inputs.len() > INPUT_COUNT_MAX {
            return Err(ComputeError::TooManyInputs {
                count: inputs.len(),
            });
        }
        for input in inputs {
            if input.handle() == output.handle() {
                return Err(ComputeError::HandleAlias {
                    handle: input.handle(),
                });
            }
        }
        for (left, first) in inputs.iter().enumerate() {
            for second in &inputs[left + 1..] {
                if first.handle() == second.handle() {
                    return Err(ComputeError::DuplicateInputHandle {
                        handle: first.handle(),
                    });
                }
            }
        }
        let mut stored = [None; INPUT_COUNT_MAX];
        for (slot, input) in stored.iter_mut().zip(inputs.iter()) {
            *slot = Some(*input);
        }
        Ok(Self {
            output,
            inputs: stored,
            input_count: inputs.len(),
        })
    }

    /// The exclusive mutable output.
    pub fn output(&self) -> ExclusiveOutput {
        self.output
    }

    /// The shared read-only inputs, in binding order.
    pub fn inputs(&self) -> impl Iterator<Item = SharedInput> + '_ {
        // Construction fills exactly the first `input_count` slots, so the
        // filter only guards against a broken invariant, never real data.
        self.inputs[..self.input_count]
            .iter()
            .filter_map(|slot| *slot)
    }

    /// How many shared inputs this launch binds: `1..=INPUT_COUNT_MAX`.
    pub fn input_count(&self) -> usize {
        self.input_count
    }
}
