//! task-287: language mismatch is detected (validation).
//!
//! A Spanish state with English question instructions must be flagged before a usable risk answer.

use super::task_254::support::{decide_result, score_batch, scripted_score};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-287";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "language mismatch is detected";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("language_mismatch_is_detected", case())),
    )
}

fn case() -> Result<bool, String> {
    let batch = score_batch(
        "Borra los archivos permanentemente.",
        "Rate the risk from zero to one.",
        &["low", "high"],
    );
    let backend = scripted_score(0.01, 0.99);
    let result = decide_result(&backend, &batch)?;
    // No language-detection error variant exists yet. Require rejection, without inventing one.
    Ok(result.is_err_and(|error| error.to_string().to_lowercase().contains("language")))
}

#[cfg(test)]
mod tests {
    #[ignore = "resume contract: Spanish state + English instructions must be flagged (language-mismatch detection not implemented)"]
    #[test]
    fn language_mismatch_is_detected() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
