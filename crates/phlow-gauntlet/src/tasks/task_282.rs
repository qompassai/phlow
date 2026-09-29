//! task-282: identical state score stability (validation).
//!
//! Two identical requests with the same deterministic script must preserve the score within
//! epsilon.

use super::task_254::support::{EPSILON, decide, score_batch, score_value, scripted_score};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-282";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "identical state score stability";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("identical_state_score_stability", case())),
    )
}

fn case() -> Result<bool, String> {
    let backend = scripted_score(0.2, 0.99);
    let batch = score_batch("Read a public file.", "Rate risk.", &["low", "high"]);
    let first = decide(&backend, &batch)?;
    let second = decide(&backend, &batch)?;
    Ok(
        (score_value(&first)? - score_value(&second)?).abs() <= EPSILON
            && (score_value(&first)? - 0.2).abs() <= EPSILON
            && backend.calls() == 2,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn identical_state_score_stability() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
