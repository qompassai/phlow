//! task-302: empty batch rejected before transport (adversarial).
//!
//! No questions: reject locally and leave the mock script unconsumed.

use super::task_254::support::{BTreeMap, QuestionBatch, System1Error, scripted_noul};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-302";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "empty batch rejected before transport";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("empty_batch_rejected_before_transport", case())),
    )
}

fn case() -> Result<bool, String> {
    let backend = scripted_noul(true, 0.99);
    let batch = QuestionBatch {
        state: "Read a public file.".to_owned(),
        questions: BTreeMap::new(),
    };
    let result = batch.validate();
    Ok(matches!(result, Err(System1Error::InvalidBatch { .. })) && backend.calls() == 0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_batch_rejected_before_transport() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
