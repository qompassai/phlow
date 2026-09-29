//! task-295: unreachable endpoint escalates (adversarial).
//!
//! Inject connection refusal; preserve the error in the escalation decision and never auto-approve.

use super::task_254::support::{
    Escalation, MockBackend, Risk, Route, System1Error, approval_decision, assess, error_text,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-295";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "unreachable endpoint escalates";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("unreachable_endpoint_escalates", case())),
    )
}

fn case() -> Result<bool, String> {
    let backend = MockBackend::failing(System1Error::Transport {
        detail: "connection refused".to_owned(),
    });
    let decision = assess(&backend, "Read a public file.", Risk::LocalReversible)?;
    if !matches!(decision,
        Route::Escalate(Escalation::System1(System1Error::Transport { detail }))
            if detail.contains("connection refused"))
    {
        return Ok(false);
    }
    let input = approval_decision(Risk::LocalReversible)?;
    let mut queue = phlow_approval::ApprovalQueue::new(&["operator"]).map_err(error_text)?;
    let id = queue
        .request(
            "system1-gauntlet",
            input.proposal(),
            phlow_approval::DEFAULT_TTL,
        )
        .map_err(error_text)?;
    let record = queue.get(&id).ok_or("fallback queue record missing")?;
    Ok(backend.calls() == 1
        && record.state == phlow_approval::State::Pending
        && record.scope == input.scope
        && record.decided_by.is_none())
}

#[cfg(test)]
mod tests {
    #[test]
    fn unreachable_endpoint_escalates() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
