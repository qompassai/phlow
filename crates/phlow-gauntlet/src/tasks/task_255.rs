//! task-255: negation in instructions (validation).
//!
//! Script a deterministic Choice answer; preserve the exact state, instructions, and option order.
//! This characterizes protocol fidelity, not the semantic accuracy of a live model.

use super::task_254::support::{check_choice, choice_batch, decide, scripted_choice};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-255";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "negation in instructions";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, std::iter::once(("negation_in_instructions", case())))
}

fn case() -> Result<bool, String> {
    let batch = choice_batch(
        "The appointment is active.",
        "Choose the action that does NOT cancel the appointment.",
        &["cancel appointment", "keep appointment"],
    );
    let backend = scripted_choice(1, 0.99);
    let answer = decide(&backend, &batch)?;
    check_choice(&backend, &batch, &answer, 1, 0.99)
}

#[cfg(test)]
mod tests {
    #[test]
    fn negation_in_instructions() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
