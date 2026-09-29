//! task-298: http 500 cause is surfaced (adversarial).
//!
//! Script an HTTP 500 failure; the escalation channel must retain its cause.

use super::task_254::support::{Escalation, MockBackend, Risk, Route, System1Error, assess};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-298";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "http 500 cause is surfaced";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, std::iter::once(("http_500_cause_is_surfaced", case())))
}

fn case() -> Result<bool, String> {
    let backend = MockBackend::failing(System1Error::Transport {
        detail: "server returned HTTP 500".to_owned(),
    });
    let decision = assess(&backend, "Read a public file.", Risk::LocalReversible)?;
    Ok(backend.calls() == 1
        && matches!(decision,
        Route::Escalate(Escalation::System1(System1Error::Transport { detail }))
            if detail.contains("500")))
}

#[cfg(test)]
mod tests {
    #[test]
    fn http_500_cause_is_surfaced() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
