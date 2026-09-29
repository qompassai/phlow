//! task-260: score negated criterion (validation).
//!
//! A negated criterion must reach the backend unchanged, with no score inversion.

use super::task_254::support::{check_score, decide, score_batch, scripted_score};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-260";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "score negated criterion";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, std::iter::once(("score_negated_criterion", case())))
}

fn case() -> Result<bool, String> {
    let batch = score_batch(
        "Read a public file.",
        "Rate risk from 0 to 1.",
        &["does NOT modify files", "irreversible modification"],
    );
    let backend = scripted_score(0.05, 0.99);
    let answer = decide(&backend, &batch)?;
    check_score(&backend, &batch, &answer, 0.05, 0.99)
}

#[cfg(test)]
mod tests {
    #[test]
    fn score_negated_criterion() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
