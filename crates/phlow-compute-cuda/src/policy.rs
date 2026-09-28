//! Compile-time kernel specialization policies.
//!
//! The idea, adapted from cuda-oxide's kernel policies: tuning choices that
//! do not change what a kernel computes — tile granularity, unroll factors,
//! tensor-core use — are fixed when the kernel is built, not passed at
//! launch. One kernel source then yields several specialized variants, each
//! identified by a deterministic variant key for caching.

use crate::error::CudaError;
use crate::kernel::KernelDescriptor;

/// Largest tile-row count a policy may request.
pub const TILE_ROWS_MAX: u32 = 64;

/// Largest unroll factor a policy may request.
pub const UNROLL_FACTOR_MAX: u32 = 16;

/// Tuning choices fixed at kernel build time.
///
/// The invariant: `tile_rows` in `1..=TILE_ROWS_MAX`, `unroll_factor` in
/// `1..=UNROLL_FACTOR_MAX`. Only [`KernelPolicy::new`] constructs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KernelPolicy {
    tile_rows: u32,
    unroll_factor: u32,
    use_tensor_cores: bool,
}

impl KernelPolicy {
    /// Builds a policy from tuning choices.
    ///
    /// # Contract
    /// - Accepts: `tile_rows` in `1..=TILE_ROWS_MAX`, `unroll_factor` in
    ///   `1..=UNROLL_FACTOR_MAX`; any `use_tensor_cores`.
    /// - Rejects: out-of-range fields — with a typed error naming the
    ///   field.
    pub fn new(
        tile_rows: u32,
        unroll_factor: u32,
        use_tensor_cores: bool,
    ) -> Result<Self, CudaError> {
        if tile_rows == 0 || tile_rows > TILE_ROWS_MAX {
            return Err(CudaError::InvalidPolicy {
                field: "tile_rows",
                value: tile_rows,
            });
        }
        if unroll_factor == 0 || unroll_factor > UNROLL_FACTOR_MAX {
            return Err(CudaError::InvalidPolicy {
                field: "unroll_factor",
                value: unroll_factor,
            });
        }
        Ok(Self {
            tile_rows,
            unroll_factor,
            use_tensor_cores,
        })
    }

    /// Tile rows per tile program.
    pub fn tile_rows(&self) -> u32 {
        self.tile_rows
    }

    /// Loop unroll factor.
    pub fn unroll_factor(&self) -> u32 {
        self.unroll_factor
    }

    /// Whether the variant may use tensor cores.
    pub fn use_tensor_cores(&self) -> bool {
        self.use_tensor_cores
    }
}

/// One specialized kernel variant: a descriptor plus its build-time policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Specialization {
    /// The kernel being specialized.
    pub descriptor: KernelDescriptor,
    /// The tuning choices for this variant.
    pub policy: KernelPolicy,
}

impl Specialization {
    /// Deterministic cache key for this variant.
    ///
    /// Field order is fixed and every field is rendered, so equal
    /// specializations always produce equal keys and different ones differ
    /// in at least one segment. The key is bounded: names and paths are
    /// already length-capped by their own types.
    pub fn variant_key(&self) -> String {
        format!(
            "name={};entry={};arch={};source={};tile_rows={};unroll={};tc={}",
            self.descriptor.name.as_str(),
            self.descriptor.entry.as_str(),
            self.descriptor.arch.name(),
            source_tag(&self.descriptor),
            self.policy.tile_rows(),
            self.policy.unroll_factor(),
            self.policy.use_tensor_cores(),
        )
    }
}

/// Short stable tag for the descriptor's code source.
fn source_tag(descriptor: &KernelDescriptor) -> &'static str {
    match descriptor.source {
        crate::kernel::KernelSource::PtxText(_) => "ptx-text",
        crate::kernel::KernelSource::PtxFile(_) => "ptx-file",
        crate::kernel::KernelSource::CubinFile(_) => "cubin-file",
    }
}
