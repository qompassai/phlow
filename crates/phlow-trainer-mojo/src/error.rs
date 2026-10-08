//! Typed errors for the Mojo scoring boundary.
//!
//! Every variant carries bounded context (shapes, limits, status codes)
//! so a caller can decide what to do. Device internals and raw buffer
//! contents never appear in messages.

use std::fmt::{Display, Formatter, Result as FmtResult};

/// Every way a scoring call can be rejected before or inside the kernel.
///
/// Rejection never produces partial output: wrappers validate first,
/// and the Mojo exports write output only on success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoringError {
    /// A shape or bound violation detected on the Rust side, before FFI.
    InvalidShape {
        /// What was wrong, in bounded human-readable form.
        detail: String,
    },
    /// The Mojo export returned a non-zero status code.
    KernelStatus {
        /// 1 = invalid argument, 2 = no accelerator, 3 = device failure.
        code: i32,
    },
    /// A driver-mode input file could not be read or parsed.
    InputFile {
        /// Path or label of the offending input.
        source: String,
        /// Bounded description of the failure.
        detail: String,
    },
}

impl Display for ScoringError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            ScoringError::InvalidShape { detail } => {
                write!(f, "invalid scoring input: {detail}")
            }
            ScoringError::KernelStatus { code } => {
                write!(f, "mojo scoring kernel returned status {code}")
            }
            ScoringError::InputFile { source, detail } => {
                write!(f, "cannot use input {source}: {detail}")
            }
        }
    }
}

impl std::error::Error for ScoringError {}
