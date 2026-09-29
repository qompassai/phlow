//! task-277: out of range index is protocol error (adversarial).
//!
//! Script index equal to option count; return a protocol error without indexing or panicking.

use super::task_254::support::{System1Error, choice_batch, decide_result, scripted_choice};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-277";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "out of range index is protocol error";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("out_of_range_index_is_protocol_error", case())),
    )
}

fn case() -> Result<bool, String> {
    let batch = choice_batch("Keep the file.", "Choose.", &["keep", "delete"]);
    let backend = scripted_choice(2, 0.99);
    let result = decide_result(&backend, &batch)?;
    Ok(matches!(result, Err(System1Error::Protocol { .. })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn out_of_range_index_is_protocol_error() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
