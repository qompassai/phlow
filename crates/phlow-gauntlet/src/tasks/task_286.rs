//! task-286: non english requires multilingual path (validation).
//!
//! Inject a confidently low risk score and a reversibility answer.
//! The independent action class and input discipline must prevent unsafe auto-approval.

use super::task_254::support::{Risk, assess, check_decision, risk_backend};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-286";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "non english requires multilingual path";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("non_english_requires_multilingual_path", case())),
    )
}

fn case() -> Result<bool, String> {
    let backend = risk_backend(0.01, 0.99, true);
    let decision = assess(
        &backend,
        "Lea el archivo sin modificarlo.",
        Risk::LocalReversible,
    )?;
    check_decision(&backend, &decision, false)
}

#[cfg(test)]
mod tests {
    #[ignore = "resume contract: non-English input requires multilingual path (language routing not implemented)"]
    #[test]
    fn non_english_requires_multilingual_path() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
