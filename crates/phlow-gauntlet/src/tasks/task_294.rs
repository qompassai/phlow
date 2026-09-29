//! task-294: thresholds require workload validation (validation).
//!
//! Explicit fixture thresholds are placeholders. The scorer must not represent them as
//! calibrated defaults.
//! A MockBackend cannot establish empirical calibration; that remains an external workload gate.

use super::task_254::support::{Risk, assess, check_decision, risk_backend};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-294";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "thresholds require workload validation";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("thresholds_require_workload_validation", case())),
    )
}

fn case() -> Result<bool, String> {
    let backend = risk_backend(0.01, 0.99, true);
    let decision = assess(&backend, "Read a public file.", Risk::LocalReversible)?;
    // Documentation is part of this task's explicit contract; this is not empirical validation.
    let docs = include_str!("../../../phlow-system1/src/risk.rs");
    Ok(check_decision(&backend, &decision, true)?
        && docs.matches("UNVALIDATED PLACEHOLDER").count() >= 2
        && docs.contains("workload"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn thresholds_require_workload_validation() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
