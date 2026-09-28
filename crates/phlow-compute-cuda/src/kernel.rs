//! Kernel descriptors: the host-side identity of one GPU kernel.
//!
//! A descriptor names the kernel, its entry point, its target architecture,
//! and where its code comes from (embedded PTX text, a PTX file, or a cubin).
//! Paths are validated for shape only — length and non-emptiness. They are
//! requests, not authorization: resolving them against the filesystem is the
//! caller's job, with its own containment policy.

use crate::arch::ComputeArch;
use crate::error::CudaError;
use crate::ptx::PtxModule;

/// Longest kernel or entry name, in bytes.
pub const NAME_BYTES_MAX: usize = 256;

/// Longest kernel artifact path, in bytes.
pub const PATH_BYTES_MAX: usize = 1024;

/// A validated kernel name: 1..=256 bytes of `[A-Za-z0-9_.]`, starting with
/// a letter or underscore.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KernelName(String);

impl KernelName {
    /// Validates a kernel name.
    pub fn new(name: &str) -> Result<Self, CudaError> {
        validate_name(name, false)?;
        Ok(Self(name.to_string()))
    }

    /// The validated name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated entry-point name: 1..=256 bytes of `[A-Za-z0-9_.$]`,
/// starting with a letter, `_`, `.`, or `$` (PTX allows the extra sigils).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryName(String);

impl EntryName {
    /// Validates an entry-point name.
    pub fn new(name: &str) -> Result<Self, CudaError> {
        validate_name(name, true)?;
        Ok(Self(name.to_string()))
    }

    /// The validated name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Shared name validation. `ptx_extra` permits `.` and `$`, which PTX entry
/// symbols may carry but kernel registry names may not.
fn validate_name(name: &str, ptx_extra: bool) -> Result<(), CudaError> {
    let bytes = name.len();
    if bytes == 0 || bytes > NAME_BYTES_MAX {
        return Err(CudaError::invalid_name(name));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap_or(' ');
    let first_ok = first.is_ascii_alphabetic()
        || first == '_'
        || (ptx_extra && (first == '.' || first == '$'));
    let rest_ok = chars
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || (ptx_extra && (c == '.' || c == '$')));
    if !first_ok || !rest_ok {
        return Err(CudaError::invalid_name(name));
    }
    Ok(())
}

/// A validated kernel artifact path: 1..=1024 bytes, non-empty.
///
/// No filesystem access happens here; containment and existence are the
/// caller's responsibility.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KernelPath(String);

impl KernelPath {
    /// Validates a path's shape.
    pub fn new(path: &str) -> Result<Self, CudaError> {
        let bytes = path.len();
        if bytes == 0 || bytes > PATH_BYTES_MAX {
            return Err(CudaError::invalid_path(path));
        }
        Ok(Self(path.to_string()))
    }

    /// The validated path text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where a kernel's code comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelSource {
    /// PTX text embedded in the descriptor (validated at construction).
    PtxText(PtxModule),
    /// PTX loaded from a file at launch time.
    PtxFile(KernelPath),
    /// A pre-compiled cubin loaded from a file at launch time.
    CubinFile(KernelPath),
}

/// The host-side identity of one GPU kernel: name, entry point, target
/// architecture, and code source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelDescriptor {
    /// Registry name, e.g. `vector_add`.
    pub name: KernelName,
    /// Device entry point, e.g. `vector_add_kernel`.
    pub entry: EntryName,
    /// Architecture the code targets.
    pub arch: ComputeArch,
    /// Where the code comes from.
    pub source: KernelSource,
}

impl KernelDescriptor {
    /// Builds a descriptor; every field is already validated by its own
    /// constructor, so this only assembles.
    pub fn new(
        name: KernelName,
        entry: EntryName,
        arch: ComputeArch,
        source: KernelSource,
    ) -> Self {
        Self {
            name,
            entry,
            arch,
            source,
        }
    }
}
