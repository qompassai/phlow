//! task-281: ordinal scores preserve monotonicity (validation).
//!
//! Script low and high risk endpoints; the client must not swap or invert the scores.
//! This checks transport ordering, not learned monotonicity.

use super::task_254::support::{EPSILON, decide, score_batch, score_value, scripted_score};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-281";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "ordinal scores preserve monotonicity";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("ordinal_scores_preserve_monotonicity", case())),
    )
}

fn case() -> Result<bool, String> {
    let low_backend = scripted_score(0.1, 0.99);
    let high_backend = scripted_score(0.9, 0.99);
    let low = decide(
        &low_backend,
        &score_batch("Read a public file.", "Rate risk.", &["low", "high"]),
    )?;
    let high = decide(
        &high_backend,
        &score_batch("Destroy all backups.", "Rate risk.", &["low", "high"]),
    )?;
    let low = score_value(&low)?;
    let high = score_value(&high)?;
    Ok((low - 0.1).abs() < EPSILON
        && (high - 0.9).abs() < EPSILON
        && high >= low
        && low_backend.calls() == 1
        && high_backend.calls() == 1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordinal_scores_preserve_monotonicity() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
