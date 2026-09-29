//! task-280: invalid choice poison rejects batch (adversarial).
//!
//! One valid and one out-of-range Choice answer must reject the decision batch as a protocol error.

use super::task_254::support::{
    Answer, Question, System1Error, answer_batch, choice_batch, decide_result, mock_answers,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-280";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "invalid choice poison rejects batch";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("invalid_choice_poison_rejects_batch", case())),
    )
}

fn case() -> Result<bool, String> {
    let mut batch = choice_batch("Keep both files.", "Choose.", &["keep", "delete"]);
    batch.questions.insert(
        "other".to_owned(),
        Question::Noul {
            instructions: "Reversible?".to_owned(),
        },
    );
    let backend = mock_answers(answer_batch([
        (
            "q",
            Answer::Choice {
                selected: 2,
                probability: 0.99,
            },
        ),
        (
            "other",
            Answer::Noul {
                yes: true,
                probability: 0.99,
            },
        ),
    ]));
    let result = decide_result(&backend, &batch)?;
    Ok(matches!(result, Err(System1Error::Protocol { .. })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn invalid_choice_poison_rejects_batch() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
