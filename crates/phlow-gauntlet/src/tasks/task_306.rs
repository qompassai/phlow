//! task-306: incomplete debug effect escalates.
//!
//! Adversarial: debug hides its network side effect from the description.
//! Script uncertainty; incomplete evidence must never become approval.
//! This tests confidence routing, not a mock model detecting omitted information.

use super::task_304::support::{RISK_ID, RiskScorer, Route, decision, ready, safe, score};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-306";
/// Defense exercised by this task.
pub const NAME: &str = "incomplete debug effect escalates";
/// Offline Rust task using a deterministic mock.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the bounded scenario and preserve failure evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match case() {
        Ok(()) => TaskOutcome::Pass {
            evidence: vec![format!("{ID}: {NAME} passed")],
        },
        Err(how) => TaskOutcome::Fail {
            where_: ID.to_owned(),
            how,
            evidence: vec![],
        },
    }
}

fn case() -> Result<(), String> {
    let mock = safe().with_answer(RISK_ID, score(0.01, 0.2));
    let scorer = RiskScorer::new(mock);
    let result = ready(scorer.route(
        &decision("debug", "observe")?,
        "Debug operation; effects unspecified.",
    ))?;
    if !matches!(
        result,
        Route::Escalate(phlow_system1::Escalation::LowConfidence { .. })
    ) {
        return Err(format!(
            "incomplete effect accepted or wrong reason: {result:?}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn incomplete_debug_effect_escalates() {
        let result = super::case();
        assert!(result.is_ok(), "{result:?}");
    }
}
