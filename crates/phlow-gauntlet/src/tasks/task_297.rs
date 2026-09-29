//! task-297: timeout discards entire batch (adversarial).
//!
//! Inject a typed timeout for a two-question batch; no partial AnswerBatch may escape.
//! MockBackend has no streaming seam: this verifies whole-batch failure, not socket timing.

use super::task_254::support::{MockBackend, Question, System1Error, decide_result, noul_batch};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-297";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "timeout discards entire batch";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("timeout_discards_entire_batch", case())),
    )
}

fn case() -> Result<bool, String> {
    let backend = MockBackend::failing(System1Error::Timeout);
    let mut batch = noul_batch("Read two public files.", "First reversible?");
    batch.questions.insert(
        "second".to_owned(),
        Question::Noul {
            instructions: "Second reversible?".to_owned(),
        },
    );
    let result = decide_result(&backend, &batch)?;
    Ok(matches!(result, Err(System1Error::Timeout)) && backend.calls() == 1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn timeout_discards_entire_batch() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
