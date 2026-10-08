//! Safe wrappers over the Mojo scoring kernels' C ABI.
//!
//! This module is the crate's only FFI boundary. All `unsafe` lives here,
//! behind wrappers that validate shapes against the crate's named bounds
//! before any pointer is formed, and that map the kernel's status codes
//! onto [`ScoringError`]. The Mojo side validates the same bounds again
//! (defense in depth across a language boundary); either layer may reject,
//! neither may partially write output.
//!
//! # Safety obligations at the boundary
//!
//! - Slices passed to the kernel are borrowed for the call only; the
//!   Mojo exports copy in, compute, and copy out synchronously, and
//!   retain no pointers (stated in the kernel module's contract).
//! - Output buffers are allocated by the wrapper, sized exactly from
//!   validated shapes, and handed over as raw pointers only for the
//!   duration of the call.
//! - Kernel status 0 is the only success; any other code is an error
//!   and the output buffer's contents are discarded.

use crate::error::ScoringError;
use crate::{
    COMPLETIONS_TOTAL_MAX, GROUP_COMPLETIONS_MAX, GROUPS_PER_LAUNCH_MAX, LOGITS_ELEMENTS_MAX,
    REPEATS_MAX, TOKENS_PER_LAUNCH_MAX, VOCAB_SIZE_MAX,
};

unsafe extern "C" {
    fn phlow_scoring_version() -> i32;
    fn phlow_logprob_token_logps(
        logits: *const f32,
        rows: i32,
        vocab: i32,
        targets: *const i32,
        out_logps: *mut f32,
    ) -> i32;
    fn phlow_logprob_bench_ns(
        logits: *const f32,
        rows: i32,
        vocab: i32,
        targets: *const i32,
        repeats: i32,
        out_total_ns: *mut i64,
    ) -> i32;
    fn phlow_rloo_advantages(
        rewards: *const f32,
        offsets: *const i32,
        groups: i32,
        out_advantages: *mut f32,
    ) -> i32;
}

/// Map a kernel status code onto the error type; 0 is success.
fn map_status(code: i32) -> Result<(), ScoringError> {
    if code == 0 {
        Ok(())
    } else {
        Err(ScoringError::KernelStatus { code })
    }
}

/// Convert a validated shape count to the ABI's i32, or reject it.
fn abi_i32(value: usize, field: &str) -> Result<i32, ScoringError> {
    i32::try_from(value).map_err(|_| ScoringError::InvalidShape {
        detail: format!("{field} = {value} exceeds the ABI i32 range"),
    })
}

/// Validate the logprob shapes shared by the scoring and bench calls.
fn validate_logprob_shapes(
    logits_len: usize,
    rows: usize,
    vocab: usize,
    targets: &[i32],
) -> Result<(), ScoringError> {
    if rows == 0 || rows > TOKENS_PER_LAUNCH_MAX as usize {
        return Err(ScoringError::InvalidShape {
            detail: format!("rows = {rows} outside 1..={TOKENS_PER_LAUNCH_MAX}"),
        });
    }
    if vocab == 0 || vocab > VOCAB_SIZE_MAX as usize {
        return Err(ScoringError::InvalidShape {
            detail: format!("vocab = {vocab} outside 1..={VOCAB_SIZE_MAX}"),
        });
    }
    let elements = rows
        .checked_mul(vocab)
        .ok_or_else(|| ScoringError::InvalidShape {
            detail: "rows * vocab overflows usize".to_string(),
        })?;
    if elements > LOGITS_ELEMENTS_MAX as usize {
        return Err(ScoringError::InvalidShape {
            detail: format!("rows * vocab = {elements} exceeds {LOGITS_ELEMENTS_MAX}"),
        });
    }
    if logits_len != elements {
        return Err(ScoringError::InvalidShape {
            detail: format!("logits length {logits_len} != rows * vocab = {elements}"),
        });
    }
    if targets.len() != rows {
        return Err(ScoringError::InvalidShape {
            detail: format!("targets length {} != rows = {rows}", targets.len()),
        });
    }
    for (row, target) in targets.iter().enumerate() {
        if *target < 0 || *target as usize >= vocab {
            return Err(ScoringError::InvalidShape {
                detail: format!("target[{row}] = {target} outside 0..{vocab}"),
            });
        }
    }
    Ok(())
}

/// The scoring ABI version reported by the Mojo library (1 today).
#[must_use]
pub fn scoring_version() -> i32 {
    // SAFETY: the export takes no arguments and reads no memory.
    unsafe { phlow_scoring_version() }
}

/// Per-token logprobs for `rows` logit rows of width `vocab`.
///
/// `logits` is row-major f32 of length `rows * vocab`; `targets` holds
/// one in-range token id per row. Returns one logprob per row.
///
/// # Errors
///
/// [`ScoringError::InvalidShape`] on any bound or length violation;
/// [`ScoringError::KernelStatus`] on a non-zero kernel status.
pub fn logprob_token_logps(
    logits: &[f32],
    rows: usize,
    vocab: usize,
    targets: &[i32],
) -> Result<Vec<f32>, ScoringError> {
    validate_logprob_shapes(logits.len(), rows, vocab, targets)?;
    let mut out = vec![0.0_f32; rows];
    // SAFETY: shapes were just validated — lengths match the ABI's
    // expectations exactly, every target is in range, and `out` has
    // room for precisely `rows` f32 values. The kernel borrows the
    // pointers for the call only and writes `out` solely on success.
    let status = unsafe {
        phlow_logprob_token_logps(
            logits.as_ptr(),
            abi_i32(rows, "rows")?,
            abi_i32(vocab, "vocab")?,
            targets.as_ptr(),
            out.as_mut_ptr(),
        )
    };
    map_status(status)?;
    Ok(out)
}

/// Re-launch the logprob kernel `repeats` times on resident data and
/// return the total launch-loop nanoseconds (enqueue + synchronize),
/// as measured inside the Mojo export.
///
/// # Errors
///
/// As [`logprob_token_logps`], plus `repeats` outside `1..=REPEATS_MAX`.
pub fn logprob_bench_ns(
    logits: &[f32],
    rows: usize,
    vocab: usize,
    targets: &[i32],
    repeats: usize,
) -> Result<i64, ScoringError> {
    validate_logprob_shapes(logits.len(), rows, vocab, targets)?;
    if repeats == 0 || repeats > REPEATS_MAX as usize {
        return Err(ScoringError::InvalidShape {
            detail: format!("repeats = {repeats} outside 1..={REPEATS_MAX}"),
        });
    }
    let mut total_ns: i64 = 0;
    // SAFETY: shapes validated as in logprob_token_logps; the kernel
    // writes exactly one i64 to total_ns on success.
    let status = unsafe {
        phlow_logprob_bench_ns(
            logits.as_ptr(),
            abi_i32(rows, "rows")?,
            abi_i32(vocab, "vocab")?,
            targets.as_ptr(),
            abi_i32(repeats, "repeats")?,
            &mut total_ns,
        )
    };
    map_status(status)?;
    Ok(total_ns)
}

/// RLOO leave-one-out advantages for the groups described by `offsets`.
///
/// `offsets` holds `groups + 1` prefix offsets into `rewards`
/// (`offsets[0] == 0`, non-decreasing, total == rewards.len()); every
/// group size must be in `2..=GROUP_COMPLETIONS_MAX`, mirroring
/// trainlab (leave-one-out is undefined below two).
///
/// # Errors
///
/// [`ScoringError::InvalidShape`] on any offset or bound violation;
/// [`ScoringError::KernelStatus`] on a non-zero kernel status.
pub fn rloo_advantages(rewards: &[f32], offsets: &[i32]) -> Result<Vec<f32>, ScoringError> {
    if offsets.len() < 2 {
        return Err(ScoringError::InvalidShape {
            detail: "offsets must describe at least one group".to_string(),
        });
    }
    let groups = offsets.len() - 1;
    if groups > GROUPS_PER_LAUNCH_MAX as usize {
        return Err(ScoringError::InvalidShape {
            detail: format!("groups = {groups} exceeds {GROUPS_PER_LAUNCH_MAX}"),
        });
    }
    if offsets[0] != 0 {
        return Err(ScoringError::InvalidShape {
            detail: "offsets[0] must be 0".to_string(),
        });
    }
    for (index, pair) in offsets.windows(2).enumerate() {
        let size = pair[1]
            .checked_sub(pair[0])
            .filter(|s| *s >= 0)
            .ok_or_else(|| ScoringError::InvalidShape {
                detail: format!("offsets decrease at group {index}"),
            })?;
        if size < 2 || size as usize > GROUP_COMPLETIONS_MAX as usize {
            return Err(ScoringError::InvalidShape {
                detail: format!("group {index} size {size} outside 2..={GROUP_COMPLETIONS_MAX}"),
            });
        }
    }
    let total = offsets[groups] as usize;
    if total != rewards.len() {
        return Err(ScoringError::InvalidShape {
            detail: format!("offsets total {total} != rewards length {}", rewards.len()),
        });
    }
    if total > COMPLETIONS_TOTAL_MAX as usize {
        return Err(ScoringError::InvalidShape {
            detail: format!("total completions {total} exceeds {COMPLETIONS_TOTAL_MAX}"),
        });
    }
    let mut out = vec![0.0_f32; total];
    // SAFETY: offsets were just validated — non-negative, monotone,
    // total == rewards.len() == out.len(); the kernel borrows the
    // pointers for the call only and writes `out` solely on success.
    let status = unsafe {
        phlow_rloo_advantages(
            rewards.as_ptr(),
            offsets.as_ptr(),
            abi_i32(groups, "groups")?,
            out.as_mut_ptr(),
        )
    };
    map_status(status)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logprob_rejects_length_mismatch() {
        let logits = vec![0.0_f32; 7];
        let targets = vec![0_i32; 2];
        assert!(matches!(
            logprob_token_logps(&logits, 2, 4, &targets),
            Err(ScoringError::InvalidShape { .. })
        ));
    }

    #[test]
    fn a_logprob_rejects_out_of_range_target() {
        let logits = vec![0.0_f32; 8];
        let targets = vec![0_i32, 4];
        assert!(matches!(
            logprob_token_logps(&logits, 2, 4, &targets),
            Err(ScoringError::InvalidShape { .. })
        ));
    }

    #[test]
    fn a_advantages_reject_singleton_group() {
        let rewards = vec![1.0_f32];
        let offsets = vec![0_i32, 1];
        assert!(matches!(
            rloo_advantages(&rewards, &offsets),
            Err(ScoringError::InvalidShape { .. })
        ));
    }

    #[test]
    fn a_advantages_reject_nonzero_first_offset() {
        let rewards = vec![1.0_f32, 0.0];
        let offsets = vec![1_i32, 2];
        assert!(matches!(
            rloo_advantages(&rewards, &offsets),
            Err(ScoringError::InvalidShape { .. })
        ));
    }
}
