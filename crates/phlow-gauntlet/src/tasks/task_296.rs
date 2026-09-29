//! task-296: malformed json has no partial application (adversarial).
//!
//! A valid-looking first answer followed by truncated JSON must produce a whole-response
//! protocol error.

use super::task_254::support::{
    AnswerBatch, Escalation, MockBackend, Risk, Route, System1Error, assess, noul_batch,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-296";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "malformed json has no partial application";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("malformed_json_has_no_partial_application", case())),
    )
}

fn case() -> Result<bool, String> {
    let batch = noul_batch("Read a public file.", "Reversible?");
    let body = br#"{"q":{"yes":true,"probability":0.99},"broken":"#;
    let parsed = AnswerBatch::from_wire(&batch, body);
    let Err(error @ System1Error::Protocol { .. }) = parsed else {
        return Ok(false);
    };
    // MockBackend scripts typed failures, not raw HTTP bytes. Drive the real parser above,
    // then pass its exact failure through the real approval fallback below.
    let backend = MockBackend::failing(error);
    let decision = assess(&backend, "Read a public file.", Risk::LocalReversible)?;
    Ok(matches!(
        decision,
        Route::Escalate(Escalation::System1(System1Error::Protocol { .. }))
    ) && backend.calls() == 1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn malformed_json_has_no_partial_application() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
