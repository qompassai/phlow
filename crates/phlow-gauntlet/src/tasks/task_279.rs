//! task-279: max index rejected without panic (adversarial).
//!
//! Script usize::MAX against 16 valid options; the client must reject the whole batch.

use super::task_254::support::{
    BTreeMap, Question, QuestionBatch, System1Error, decide_result, scripted_choice,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-279";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "max index rejected without panic";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("max_index_rejected_without_panic", case())),
    )
}

fn case() -> Result<bool, String> {
    let options: Vec<String> = (0..16).map(|index| format!("option-{index}")).collect();
    let batch = QuestionBatch {
        state: "Select option-0.".to_owned(),
        questions: BTreeMap::from([(
            "q".to_owned(),
            Question::Choice {
                instructions: "Choose one.".to_owned(),
                options,
            },
        )]),
    };
    let backend = scripted_choice(usize::MAX, 0.99);
    let result = decide_result(&backend, &batch)?;
    Ok(matches!(result, Err(System1Error::Protocol { .. })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn max_index_rejected_without_panic() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
