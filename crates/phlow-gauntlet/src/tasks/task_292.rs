//! task-292: threshold four quadrants (validation).
//!
//! Exercise all four combinations around both thresholds on an independently low-risk
//! reversible action.

use super::task_254::support::{
    EPSILON, MIN_CONFIDENCE, RISK_THRESHOLD, Risk, assess, check_decision, risk_backend,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-292";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "threshold four quadrants";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, std::iter::once(("threshold_four_quadrants", case())))
}

fn case() -> Result<bool, String> {
    for (risk, confidence, expected) in [
        (RISK_THRESHOLD - EPSILON, MIN_CONFIDENCE + EPSILON, true),
        (RISK_THRESHOLD - EPSILON, MIN_CONFIDENCE - EPSILON, false),
        (RISK_THRESHOLD + EPSILON, MIN_CONFIDENCE + EPSILON, false),
        (RISK_THRESHOLD + EPSILON, MIN_CONFIDENCE - EPSILON, false),
    ] {
        let backend = risk_backend(risk, confidence, true);
        let decision = assess(&backend, "Read a public file.", Risk::LocalReversible)?;
        if !check_decision(&backend, &decision, expected)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn threshold_four_quadrants() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
