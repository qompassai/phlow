//! task-301: malformed answer isolated in valid envelope (adversarial).
//!
//! A syntactically valid response has one wrong answer type. Preserve the good answer for
//! inspection.
//! Unlike invalid Choice indices (task-280) and broken JSON (task-296), this is recoverable for
//! inspection only.

use super::task_254::support::{
    Answer, AnswerBatch, BTreeMap, Question, QuestionBatch, System1Decider, answer_batch,
    error_text, mock_answers, ready,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-301";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "malformed answer isolated in valid envelope";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("malformed_answer_isolated_in_valid_envelope", case())),
    )
}

fn case() -> Result<bool, String> {
    let batch = QuestionBatch {
        state: "Inspect two facts; authorize nothing.".to_owned(),
        questions: BTreeMap::from([
            (
                "good".to_owned(),
                Question::Noul {
                    instructions: "Is this a read?".to_owned(),
                },
            ),
            (
                "bad".to_owned(),
                Question::Score {
                    instructions: "Rate risk.".to_owned(),
                    criteria: vec!["low".to_owned(), "high".to_owned()],
                },
            ),
        ]),
    };
    // A wrong-kind typed answer is malformed against its question. MockBackend deliberately
    // returns it unvalidated, enabling inspection before any authorization is attempted.
    let backend = mock_answers(answer_batch([
        (
            "good",
            Answer::Noul {
                yes: true,
                probability: 0.99,
            },
        ),
        (
            "bad",
            Answer::Choice {
                selected: 0,
                probability: 0.99,
            },
        ),
    ]));
    let raw = ready(backend.decide(&batch))?.map_err(error_text)?;
    let wire = raw.to_wire().map_err(error_text)?;
    let decoded = AnswerBatch::from_wire(&batch, &wire);
    // Desired partial-inspection contract: the good answer survives the real decoding seam.
    // Current from_wire rejects the entire response, so this is expected to fail.
    Ok(matches!(decoded, Ok(ref answers)
        if matches!(answers.answers.get("good"), Some(Answer::Noul { yes: true, .. }))
            && !answers.answers.contains_key("bad"))
        && backend.calls() == 1)
}

#[cfg(test)]
mod tests {
    #[ignore = "resume contract: malformed answer must be isolated, good answer preserved (partial-inspection API not implemented)"]
    #[test]
    fn malformed_answer_isolated_in_valid_envelope() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
