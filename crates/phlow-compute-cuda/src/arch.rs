//! Compute architectures and PTX ISA versions.
//!
//! The set is deliberately small: only architectures the simulator and the
//! validators promise to reason about. Adding one is a product decision,
//! not a string change, so the type is exhaustive.

use crate::error::CudaError;

/// A GPU compute architecture this crate models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComputeArch {
    /// NVIDIA Ampere, e.g. A100.
    Sm80,
    /// NVIDIA Hopper, e.g. H100.
    Sm90,
    /// NVIDIA Blackwell, e.g. B200.
    Sm100,
}

impl ComputeArch {
    /// Parses an architecture name.
    ///
    /// # Contract
    /// - Accepts (case-insensitive, surrounding whitespace ignored):
    ///   `sm_80`/`sm80`/`compute_80`/`80`, and the same families for 90 and
    ///   100.
    /// - Rejects: anything else — with [`CudaError::UnknownArch`] carrying
    ///   bounded text.
    pub fn parse(text: &str) -> Result<Self, CudaError> {
        match text.trim().to_ascii_lowercase().as_str() {
            "sm_80" | "sm80" | "compute_80" | "80" => Ok(Self::Sm80),
            "sm_90" | "sm90" | "compute_90" | "90" => Ok(Self::Sm90),
            "sm_100" | "sm100" | "compute_100" | "100" => Ok(Self::Sm100),
            _ => Err(CudaError::unknown_arch(text)),
        }
    }

    /// Canonical short name: `sm_80`, `sm_90`, `sm_100`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sm80 => "sm_80",
            Self::Sm90 => "sm_90",
            Self::Sm100 => "sm_100",
        }
    }
}

/// A PTX ISA version, as written in a `.version` directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PtxVersion {
    /// Major ISA version.
    pub major: u8,
    /// Minor ISA version.
    pub minor: u8,
}

impl PtxVersion {
    /// Oldest ISA this crate validates: 7.0.
    pub const MIN: Self = Self { major: 7, minor: 0 };

    /// Newest ISA this crate validates: 9.9.
    pub const MAX: Self = Self { major: 9, minor: 9 };

    /// Parses `major.minor` from a `.version` directive payload.
    ///
    /// # Contract
    /// - Accepts: two dot-separated decimal numbers inside the
    ///   [`PtxVersion::MIN`]..=[`PtxVersion::MAX`] range.
    /// - Rejects: malformed text or out-of-range versions — with a typed
    ///   error carrying bounded text.
    pub fn parse(text: &str) -> Result<Self, CudaError> {
        let trimmed = text.trim();
        let (major_text, minor_text) = trimmed
            .split_once('.')
            .ok_or_else(|| CudaError::ptx_version_malformed(text))?;
        let major: u8 = major_text
            .parse()
            .map_err(|_| CudaError::ptx_version_malformed(text))?;
        let minor: u8 = minor_text
            .parse()
            .map_err(|_| CudaError::ptx_version_malformed(text))?;
        let version = Self { major, minor };
        if version < Self::MIN || version > Self::MAX {
            return Err(CudaError::PtxVersionOutOfRange { major, minor });
        }
        Ok(version)
    }
}
