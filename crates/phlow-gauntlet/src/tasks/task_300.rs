//! task-300: ten question batch preserves ids (validation).
//!
//! Ten different scripted answers must return under the original IDs in one backend call.

use super::task_254::support::{
    Answer, AnswerBatch, BTreeMap, Question, QuestionBatch, decide, error_text, mock_answers,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-300";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "ten question batch preserves ids";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("ten_question_batch_preserves_ids", case())),
    )
}

fn case() -> Result<bool, String> {
    let mut questions = BTreeMap::new();
    let mut answers = BTreeMap::new();
    for index in 0..10 {
        let id = format!("q-{index:02}");
        questions.insert(
            id.clone(),
            Question::Choice {
                instructions: format!("Select {index}."),
                options: (0..10).map(|option| format!("option-{option}")).collect(),
            },
        );
        answers.insert(
            id,
            Answer::Choice {
                selected: index,
                probability: 0.99,
            },
        );
    }
    let expected = AnswerBatch { answers };
    let backend = mock_answers(expected.clone());
    let batch = QuestionBatch {
        state: "Ten independent selections.".to_owned(),
        questions,
    };
    let actual = decide(&backend, &batch)?;
    let wire = actual.to_wire().map_err(error_text)?;
    let decoded = AnswerBatch::from_wire(&batch, &wire).map_err(error_text)?;
    Ok(decoded == expected
        && actual == expected
        && backend.calls() == 1
        && backend.last_batch() == Some(batch))
}

#[cfg(test)]
mod tests {
    #[test]
    fn ten_question_batch_preserves_ids() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
