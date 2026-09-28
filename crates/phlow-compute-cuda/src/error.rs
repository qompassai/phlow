//! Typed errors for the CUDA kernel-target model.
//!
//! Every variant carries bounded context: counts, sizes, and truncated
//! names. PTX text and kernel binaries never appear in an error.

use std::fmt;

/// Longest string reproduced inside an error message, in characters.
const ERROR_TEXT_CHARS_MAX: usize = 64;

/// Truncates `text` for error context. Private: only this module builds the
/// bounded strings the variants carry.
fn bound_text(text: &str) -> String {
    text.chars().take(ERROR_TEXT_CHARS_MAX).collect()
}

/// Failures from validating architectures, PTX, descriptors, launch
/// configs, policies, descriptor files, and simulated-backend operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CudaError {
    /// An architecture string named nothing this crate models.
    UnknownArch {
        /// The rejected text, truncated.
        text: String,
    },
    /// PTX text exceeded [`crate::PTX_BYTES_MAX`].
    PtxTooLarge {
        /// Rejected byte length.
        bytes: usize,
    },
    /// PTX text carried no `.version` directive.
    PtxMissingVersion,
    /// PTX text carried no `.target` directive.
    PtxMissingTarget,
    /// The `.version` directive was outside the supported ISA range.
    PtxVersionOutOfRange {
        /// Rejected major version.
        major: u8,
        /// Rejected minor version.
        minor: u8,
    },
    /// A `.version` directive did not parse as `major.minor`.
    PtxVersionMalformed {
        /// The rejected directive text, truncated.
        text: String,
    },
    /// A kernel or entry name broke the charset/length rules.
    InvalidName {
        /// The rejected name, truncated.
        name: String,
    },
    /// A kernel artifact path was empty or too long.
    InvalidPath {
        /// The rejected path, truncated.
        path: String,
    },
    /// A launch dimension was zero.
    ZeroDim {
        /// Which logical dimension: `grid.x`, `block.y`, and so on.
        what: &'static str,
    },
    /// A grid axis exceeded `2^31 - 1`.
    GridDimTooLarge {
        /// Which axis.
        what: &'static str,
        /// The rejected value.
        value: u32,
    },
    /// A thread block held more than [`crate::BLOCK_THREADS_MAX`] threads.
    BlockThreadsTooMany {
        /// The rejected thread count.
        threads: u64,
    },
    /// The block thread product overflowed `u64`.
    BlockThreadsOverflow,
    /// The launch asked for more dynamic shared memory than allowed.
    SharedMemTooLarge {
        /// The rejected byte count.
        bytes: u32,
    },
    /// A kernel policy field was outside its range.
    InvalidPolicy {
        /// Which field.
        field: &'static str,
        /// The rejected value.
        value: u32,
    },
    /// A launch bound too many device pointers or scalar words.
    TooManyArgs {
        /// Which list.
        what: &'static str,
        /// The rejected count.
        count: usize,
    },
    /// A simulator allocation asked for zero bytes or an absurd size.
    InvalidAllocSize {
        /// The rejected byte count.
        bytes: u64,
    },
    /// A simulator allocation would exceed the device budget.
    OutOfMemory {
        /// Requested bytes.
        requested: u64,
        /// Bytes still free.
        available: u64,
    },
    /// An unknown allocation id was referenced.
    UnknownAlloc {
        /// The unknown id.
        id: u64,
    },
    /// An allocation was freed twice.
    DoubleFree {
        /// The already-freed id.
        id: u64,
    },
    /// A host/device copy size did not match the allocation.
    CopySizeMismatch {
        /// Allocation size in bytes.
        alloc_bytes: u64,
        /// Copy size in bytes.
        copy_bytes: u64,
    },
    /// No behavior was registered for the launched kernel name.
    UnknownKernel {
        /// The kernel name, truncated.
        name: String,
    },
    /// A registered simulator behavior reported failure.
    KernelBehaviorFailed {
        /// The kernel name, truncated.
        name: String,
        /// The behavior's static reason; never device memory contents.
        detail: &'static str,
    },
    /// A descriptor file exceeded [`crate::DESCRIPTOR_BYTES_MAX`].
    DescriptorTooLarge {
        /// Rejected byte length.
        bytes: usize,
    },
    /// A descriptor file carried too many lines.
    TooManyDescriptorLines {
        /// Rejected line count.
        lines: usize,
    },
    /// A descriptor line exceeded the per-line bound.
    DescriptorLineTooLong {
        /// 1-based line number.
        line: usize,
    },
    /// A descriptor line was not `key = value`.
    DescriptorSyntax {
        /// 1-based line number.
        line: usize,
    },
    /// A descriptor key is not part of the format.
    UnknownDescriptorKey {
        /// 1-based line number.
        line: usize,
        /// The rejected key, truncated.
        key: String,
    },
    /// A descriptor key appeared twice.
    DuplicateDescriptorKey {
        /// The duplicated key.
        key: &'static str,
    },
    /// A required descriptor key was absent.
    MissingDescriptorKey {
        /// The absent key.
        key: &'static str,
    },
    /// A descriptor value failed its own validation.
    InvalidDescriptorValue {
        /// The offending key.
        key: &'static str,
    },
}

impl CudaError {
    /// Builds [`CudaError::UnknownArch`] with bounded text.
    pub fn unknown_arch(text: &str) -> Self {
        Self::UnknownArch {
            text: bound_text(text),
        }
    }

    /// Builds [`CudaError::InvalidName`] with bounded text.
    pub fn invalid_name(name: &str) -> Self {
        Self::InvalidName {
            name: bound_text(name),
        }
    }

    /// Builds [`CudaError::InvalidPath`] with bounded text.
    pub fn invalid_path(path: &str) -> Self {
        Self::InvalidPath {
            path: bound_text(path),
        }
    }

    /// Builds [`CudaError::PtxVersionMalformed`] with bounded text.
    pub fn ptx_version_malformed(text: &str) -> Self {
        Self::PtxVersionMalformed {
            text: bound_text(text),
        }
    }

    /// Builds [`CudaError::UnknownKernel`] with bounded text.
    pub fn unknown_kernel(name: &str) -> Self {
        Self::UnknownKernel {
            name: bound_text(name),
        }
    }

    /// Builds [`CudaError::KernelBehaviorFailed`] with bounded text.
    pub fn kernel_behavior_failed(name: &str, detail: &'static str) -> Self {
        Self::KernelBehaviorFailed {
            name: bound_text(name),
            detail,
        }
    }

    /// Builds [`CudaError::UnknownDescriptorKey`] with bounded text.
    pub fn unknown_descriptor_key(line: usize, key: &str) -> Self {
        Self::UnknownDescriptorKey {
            line,
            key: bound_text(key),
        }
    }
}

impl fmt::Display for CudaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownArch { text } => {
                write!(formatter, "unknown compute architecture '{text}'")
            }
            Self::PtxTooLarge { bytes } => {
                write!(
                    formatter,
                    "PTX text of {bytes} bytes exceeds the module bound"
                )
            }
            Self::PtxMissingVersion => {
                write!(formatter, "PTX text carries no .version directive")
            }
            Self::PtxMissingTarget => {
                write!(formatter, "PTX text carries no .target directive")
            }
            Self::PtxVersionOutOfRange { major, minor } => {
                write!(
                    formatter,
                    "PTX version {major}.{minor} is outside the supported range"
                )
            }
            Self::PtxVersionMalformed { text } => {
                write!(formatter, "malformed PTX .version directive '{text}'")
            }
            Self::InvalidName { name } => {
                write!(formatter, "invalid kernel/entry name '{name}'")
            }
            Self::InvalidPath { path } => {
                write!(formatter, "invalid kernel artifact path '{path}'")
            }
            Self::ZeroDim { what } => {
                write!(formatter, "launch dimension {what} must be at least 1")
            }
            Self::GridDimTooLarge { what, value } => {
                write!(
                    formatter,
                    "grid dimension {what} of {value} exceeds 2^31 - 1"
                )
            }
            Self::BlockThreadsTooMany { threads } => {
                write!(
                    formatter,
                    "block of {threads} threads exceeds the 1024-thread limit"
                )
            }
            Self::BlockThreadsOverflow => {
                write!(formatter, "block thread product overflowed u64")
            }
            Self::SharedMemTooLarge { bytes } => {
                write!(
                    formatter,
                    "shared memory request of {bytes} bytes exceeds the bound"
                )
            }
            Self::InvalidPolicy { field, value } => {
                write!(
                    formatter,
                    "kernel policy field {field} rejects value {value}"
                )
            }
            Self::TooManyArgs { what, count } => {
                write!(
                    formatter,
                    "launch argument list {what} of {count} exceeds its bound"
                )
            }
            Self::InvalidAllocSize { bytes } => {
                write!(formatter, "invalid device allocation size {bytes}")
            }
            Self::OutOfMemory {
                requested,
                available,
            } => write!(
                formatter,
                "device allocation of {requested} bytes exceeds {available} free bytes"
            ),
            Self::UnknownAlloc { id } => {
                write!(formatter, "unknown device allocation id {id}")
            }
            Self::DoubleFree { id } => {
                write!(formatter, "device allocation {id} was already freed")
            }
            Self::CopySizeMismatch {
                alloc_bytes,
                copy_bytes,
            } => write!(
                formatter,
                "copy of {copy_bytes} bytes does not match allocation of {alloc_bytes} bytes"
            ),
            Self::UnknownKernel { name } => {
                write!(formatter, "no behavior registered for kernel '{name}'")
            }
            Self::KernelBehaviorFailed { name, detail } => {
                write!(formatter, "simulated kernel '{name}' failed: {detail}")
            }
            Self::DescriptorTooLarge { bytes } => {
                write!(
                    formatter,
                    "descriptor file of {bytes} bytes exceeds the bound"
                )
            }
            Self::TooManyDescriptorLines { lines } => {
                write!(
                    formatter,
                    "descriptor file of {lines} lines exceeds the bound"
                )
            }
            Self::DescriptorLineTooLong { line } => {
                write!(
                    formatter,
                    "descriptor line {line} exceeds the per-line bound"
                )
            }
            Self::DescriptorSyntax { line } => {
                write!(formatter, "descriptor line {line} is not 'key = value'")
            }
            Self::UnknownDescriptorKey { line, key } => {
                write!(formatter, "descriptor line {line} has unknown key '{key}'")
            }
            Self::DuplicateDescriptorKey { key } => {
                write!(formatter, "descriptor key '{key}' appears twice")
            }
            Self::MissingDescriptorKey { key } => {
                write!(formatter, "descriptor is missing required key '{key}'")
            }
            Self::InvalidDescriptorValue { key } => {
                write!(formatter, "descriptor value for key '{key}' is invalid")
            }
        }
    }
}

impl std::error::Error for CudaError {}
