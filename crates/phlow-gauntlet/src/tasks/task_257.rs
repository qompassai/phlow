//! task-257: negation scoped to one action (validation).
//!
//! Script a deterministic Choice answer; preserve the exact state, instructions, and option order.
//! This characterizes protocol fidelity, not the semantic accuracy of a live model.

use super::task_254::support::{check_choice, choice_batch, decide, scripted_choice};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-257";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "negation scoped to one action";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("negation_scoped_to_one_action", case())),
    )
}

fn case() -> Result<bool, String> {
    let batch = choice_batch(
        "Do not cancel appointment A; cancel appointment B.",
        "Which appointment should be cancelled?",
        &["A", "B", "both", "neither"],
    );
    let backend = scripted_choice(1, 0.99);
    let answer = decide(&backend, &batch)?;
    check_choice(&backend, &batch, &answer, 1, 0.99)
}

#[cfg(test)]
mod tests {
    #[test]
    fn negation_scoped_to_one_action() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
