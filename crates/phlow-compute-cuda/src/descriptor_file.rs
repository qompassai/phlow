//! Kernel descriptor files: a small text format for naming kernels.
//!
//! Format (see `kernels/cuda/*.desc` for examples):
//!
//! ```text
//! # comment lines and blank lines are ignored
//! name = vector_add
//! entry = vector_add_kernel
//! arch = sm_80
//! source = ptx:kernels/ptx/valid_add.ptx
//! ```
//!
//! Keys are exactly `name`, `entry`, `arch`, `source`; each must appear
//! once. `source` is `ptx:<path>` or `cubin:<path>`. Values are validated
//! by the same constructors the programmatic API uses, so a file cannot
//! smuggle in a descriptor the API would reject.

use crate::arch::ComputeArch;
use crate::error::CudaError;
use crate::kernel::{EntryName, KernelDescriptor, KernelName, KernelPath, KernelSource};

/// Largest descriptor file accepted, in bytes.
pub const DESCRIPTOR_BYTES_MAX: usize = 64 * 1024;

/// Most lines a descriptor file may carry.
pub const DESCRIPTOR_LINES_MAX: usize = 256;

/// Longest single descriptor line, in bytes.
pub const DESCRIPTOR_LINE_BYTES_MAX: usize = 1024;

/// Parses a descriptor file into a [`KernelDescriptor`].
///
/// # Contract
/// - Accepts: text within the byte/line bounds, with each required key
///   present exactly once and every value passing its own validation.
/// - Rejects: oversized input, over-long lines, malformed `key = value`
///   lines, unknown or duplicated keys, missing keys, and invalid values —
///   with a typed error naming the 1-based line where parsing stopped.
pub fn parse_descriptor_file(text: &str) -> Result<KernelDescriptor, CudaError> {
    if text.len() > DESCRIPTOR_BYTES_MAX {
        return Err(CudaError::DescriptorTooLarge { bytes: text.len() });
    }
    let mut name: Option<&str> = None;
    let mut entry: Option<&str> = None;
    let mut arch: Option<&str> = None;
    let mut source: Option<&str> = None;
    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line_number > DESCRIPTOR_LINES_MAX {
            return Err(CudaError::TooManyDescriptorLines { lines: line_number });
        }
        if raw_line.len() > DESCRIPTOR_LINE_BYTES_MAX {
            return Err(CudaError::DescriptorLineTooLong { line: line_number });
        }
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) =
            split_key_value(line).ok_or(CudaError::DescriptorSyntax { line: line_number })?;
        let slot = match key {
            "name" => &mut name,
            "entry" => &mut entry,
            "arch" => &mut arch,
            "source" => &mut source,
            _ => return Err(CudaError::unknown_descriptor_key(line_number, key)),
        };
        if slot.is_some() {
            return Err(CudaError::DuplicateDescriptorKey {
                key: static_key(key),
            });
        }
        *slot = Some(value);
    }
    let name = name.ok_or(CudaError::MissingDescriptorKey { key: "name" })?;
    let entry = entry.ok_or(CudaError::MissingDescriptorKey { key: "entry" })?;
    let arch = arch.ok_or(CudaError::MissingDescriptorKey { key: "arch" })?;
    let source = source.ok_or(CudaError::MissingDescriptorKey { key: "source" })?;
    let descriptor = KernelDescriptor::new(
        KernelName::new(name).map_err(|_| CudaError::InvalidDescriptorValue { key: "name" })?,
        EntryName::new(entry).map_err(|_| CudaError::InvalidDescriptorValue { key: "entry" })?,
        ComputeArch::parse(arch).map_err(|_| CudaError::InvalidDescriptorValue { key: "arch" })?,
        parse_source(source)?,
    );
    Ok(descriptor)
}

/// Splits one `key = value` line. Returns `None` when there is no `=`, the
/// key is empty, or the value is empty after trimming.
fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key, value))
}

/// Maps a known key to its static spelling for error context. Only called
/// with keys that already matched, so the fallback is unreachable.
fn static_key(key: &str) -> &'static str {
    match key {
        "name" => "name",
        "entry" => "entry",
        "arch" => "arch",
        "source" => "source",
        _ => "unknown",
    }
}

/// Parses the `source` value: `ptx:<path>` or `cubin:<path>`.
fn parse_source(value: &str) -> Result<KernelSource, CudaError> {
    if let Some(path) = value.strip_prefix("ptx:") {
        let path = KernelPath::new(path.trim())
            .map_err(|_| CudaError::InvalidDescriptorValue { key: "source" })?;
        return Ok(KernelSource::PtxFile(path));
    }
    if let Some(path) = value.strip_prefix("cubin:") {
        let path = KernelPath::new(path.trim())
            .map_err(|_| CudaError::InvalidDescriptorValue { key: "source" })?;
        return Ok(KernelSource::CubinFile(path));
    }
    Err(CudaError::InvalidDescriptorValue { key: "source" })
}
