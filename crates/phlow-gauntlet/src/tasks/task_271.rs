//! task-271: oversized state fails before backend (adversarial).
//!
//! Offer one MiB of state, far above the context discipline; reject before consuming any script.

use super::task_254::support::{System1Error, noul_batch, scripted_noul};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-271";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "oversized state fails before backend";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("oversized_state_fails_before_backend", case())),
    )
}

fn case() -> Result<bool, String> {
    const OVERSIZED_STATE_BYTES: usize = 1_048_576;
    let batch = noul_batch(&"x".repeat(OVERSIZED_STATE_BYTES), "Is this reversible?");
    let backend = scripted_noul(true, 0.99);
    let result = batch.validate();
    Ok(matches!(result, Err(System1Error::InvalidBatch { .. })) && backend.calls() == 0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn oversized_state_fails_before_backend() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
