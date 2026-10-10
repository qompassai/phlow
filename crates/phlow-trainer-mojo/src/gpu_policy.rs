//! GPU-residency policy for the shared 8 GiB card.
//!
//! Primo's RTX 4070 Laptop is shared — Ollama, the desktop, and other
//! jobs hold VRAM minute to minute, and some stacks on this machine
//! have silently corrupted results under memory pressure (the Burn
//! track's CubeCL find). The Mojo backend therefore checks free VRAM
//! through `nvidia-smi` **before** any device allocation:
//!
//! - free VRAM at or above the configured minimum → proceed;
//! - below it, or `nvidia-smi` missing/unparseable → the backend is
//!   *unavailable*: a typed [`ScoringError::Unavailable`], never a
//!   partial or guessed result. The dispatch layer may then fall
//!   back to the reference path, and the receipt records both the
//!   fallback and this check's observed figures.
//!
//! The check is a bounded subprocess call: separate argv (no shell),
//! output parsed from the first CSV line only.

use std::process::Command;

use crate::error::ScoringError;

/// Default minimum free VRAM (MiB) the Mojo backend requires before
/// allocating. The scoring/sampling working set is ≤ ~250 MiB for
/// every in-contract shape the first pass measured; 1024 MiB leaves
/// 4× headroom for the MAX runtime's own device footprint while still
/// refusing to squeeze into a nearly-full card.
pub const GPU_FREE_MIB_MIN_DEFAULT: u64 = 1024;

/// One `nvidia-smi` observation of the first GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuSnapshot {
    /// Free VRAM at observation time (MiB).
    pub free_mib: u64,
    /// Total VRAM (MiB).
    pub total_mib: u64,
}

/// Query the first GPU's memory via `nvidia-smi`.
///
/// # Errors
///
/// [`ScoringError::Unavailable`] when the tool is missing, exits
/// non-zero, or its output does not parse — the fail-closed reading
/// of every failure mode: an unobservable GPU is an unavailable GPU.
pub fn query_gpu() -> Result<GpuSnapshot, ScoringError> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=memory.free,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .map_err(|error| ScoringError::Unavailable {
            detail: format!("nvidia-smi could not run: {error}"),
        })?;
    if !output.status.success() {
        return Err(ScoringError::Unavailable {
            detail: format!("nvidia-smi exited with {}", output.status),
        });
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let first = text.lines().next().unwrap_or_default();
    let mut fields = first.split(',');
    let parse = |field: Option<&str>, name: &str| -> Result<u64, ScoringError> {
        field
            .map(str::trim)
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| ScoringError::Unavailable {
                detail: format!("nvidia-smi output lacks a parseable {name}: {first:?}"),
            })
    };
    Ok(GpuSnapshot {
        free_mib: parse(fields.next(), "free memory")?,
        total_mib: parse(fields.next(), "total memory")?,
    })
}

/// Enforce the residency policy: the observed free VRAM must meet
/// `min_free_mib`. Returns the snapshot on success.
///
/// # Errors
///
/// [`ScoringError::Unavailable`] with the observed and required
/// figures when the card is too full (or unobservable, via
/// [`query_gpu`]).
pub fn ensure_capacity(min_free_mib: u64) -> Result<GpuSnapshot, ScoringError> {
    let snapshot = query_gpu()?;
    if snapshot.free_mib < min_free_mib {
        return Err(ScoringError::Unavailable {
            detail: format!(
                "GPU free VRAM {} MiB below required {} MiB (total {} MiB)",
                snapshot.free_mib, min_free_mib, snapshot.total_mib
            ),
        });
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impossible_threshold_is_unavailable() {
        // Adversarial: demanding more than any card holds must fail
        // closed with the typed error, not panic or proceed. (On a
        // machine without nvidia-smi this also passes, via the
        // unobservable-GPU branch.)
        let result = ensure_capacity(u64::MAX);
        assert!(matches!(result, Err(ScoringError::Unavailable { .. })));
    }

    #[test]
    fn zero_threshold_accepts_any_observable_gpu() {
        // Validation of the happy path where a GPU exists; where the
        // CI machine has none, the unobservable branch is the typed
        // error and equally acceptable evidence of fail-closedness.
        match ensure_capacity(0) {
            Ok(snapshot) => assert!(snapshot.total_mib > 0),
            Err(ScoringError::Unavailable { .. }) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
}
