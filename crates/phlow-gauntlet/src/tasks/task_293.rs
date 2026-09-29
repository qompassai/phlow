//! task-293: joint ties and nonfinite scores fail closed (adversarial).
//!
//! Joint threshold ties and non-finite score/confidence values must never auto-approve.

use super::task_254::support::{
    MIN_CONFIDENCE, RISK_THRESHOLD, Risk, assess, check_decision, risk_backend,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-293";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "joint ties and nonfinite scores fail closed";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("joint_ties_and_nonfinite_scores_fail_closed", case())),
    )
}

fn case() -> Result<bool, String> {
    for (risk, confidence) in [
        (RISK_THRESHOLD, MIN_CONFIDENCE),
        (f64::NAN, 0.99),
        (0.01, f64::NAN),
        (f64::NEG_INFINITY, 0.99),
        (0.01, f64::INFINITY),
    ] {
        let backend = risk_backend(risk, confidence, true);
        let decision = assess(&backend, "Read a public file.", Risk::LocalReversible)?;
        if !check_decision(&backend, &decision, false)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn joint_ties_and_nonfinite_scores_fail_closed() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
