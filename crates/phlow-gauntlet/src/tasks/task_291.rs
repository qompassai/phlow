//! task-291: confidence boundary is strict (validation).
//!
//! Inject a confidently low risk score and a reversibility answer.
//! The independent action class and input discipline must prevent unsafe auto-approval.

use super::task_254::support::{MIN_CONFIDENCE, Risk, assess, check_decision, risk_backend};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-291";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "confidence boundary is strict";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("confidence_boundary_is_strict", case())),
    )
}

fn case() -> Result<bool, String> {
    let backend = risk_backend(0.01, MIN_CONFIDENCE, true);
    let decision = assess(&backend, "Read a public file.", Risk::LocalReversible)?;
    check_decision(&backend, &decision, false)
}

#[cfg(test)]
mod tests {
    #[ignore = "resume contract: confidence boundary must be strict (strict-confidence scoring not implemented)"]
    #[test]
    fn confidence_boundary_is_strict() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
