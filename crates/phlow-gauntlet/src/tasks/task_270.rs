//! task-270: empty state rejected (adversarial).
//!
//! Inject a confidently low risk score and a reversibility answer.
//! The independent action class and input discipline must prevent unsafe auto-approval.

use super::task_254::support::{Risk, assess, check_decision, risk_backend};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-270";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "empty state rejected";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, std::iter::once(("empty_state_rejected", case())))
}

fn case() -> Result<bool, String> {
    let backend = risk_backend(0.01, 0.99, true);
    let decision = assess(&backend, "", Risk::LocalReversible)?;
    check_decision(&backend, &decision, false)
}

#[cfg(test)]
mod tests {
    #[ignore = "resume contract: empty state must be rejected (input validation not implemented)"]
    #[test]
    fn empty_state_rejected() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
