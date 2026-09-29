//! task-303: duplicate ids have deterministic map semantics (validation).
//!
//! BTreeMap cannot carry duplicate keys: inserting the same ID replaces the old question.
//! Assert the backend sees only the final question and returns one answer; raw JSON
//! duplicate-key rejection is a separate contract.

use super::task_254::support::{
    BTreeMap, Question, QuestionBatch, check_noul, decide, scripted_noul,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-303";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "duplicate ids have deterministic map semantics";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("duplicate_ids_have_deterministic_map_semantics", case())),
    )
}

fn case() -> Result<bool, String> {
    let mut questions = BTreeMap::new();
    questions.insert(
        "q".to_owned(),
        Question::Noul {
            instructions: "Old question.".to_owned(),
        },
    );
    let replaced = questions.insert(
        "q".to_owned(),
        Question::Noul {
            instructions: "Final question.".to_owned(),
        },
    );
    let batch = QuestionBatch {
        state: "Read a public file.".to_owned(),
        questions,
    };
    let backend = scripted_noul(true, 0.99);
    let answer = decide(&backend, &batch)?;
    Ok(replaced.is_some()
        && batch.questions.len() == 1
        && answer.answers.len() == 1
        && check_noul(&backend, &batch, &answer, true, 0.99)?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn duplicate_ids_have_deterministic_map_semantics() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
