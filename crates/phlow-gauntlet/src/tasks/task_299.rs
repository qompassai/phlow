//! task-299: alternating transport does not flap (adversarial).
//!
//! Success/failure/success scripts must consume exactly one batch per decision, with no hidden
//! retry or split question calls.

use super::task_254::support::{
    AnswerBatch, Escalation, MockBackend, QuestionBatch, Risk, RiskScorer, Route, System1Decider,
    System1Error, approval_decision, ready, risk_backend,
};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-299";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "alternating transport does not flap";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one adversarial scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(
        ID,
        std::iter::once(("alternating_transport_does_not_flap", case())),
    )
}

fn case() -> Result<bool, String> {
    let backend = AlternatingMocks {
        success: risk_backend(0.01, 0.99, true),
        failure: MockBackend::failing(System1Error::Transport {
            detail: "intermittent failure".to_owned(),
        }),
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    let scorer = RiskScorer::new(backend);
    let input = approval_decision(Risk::LocalReversible)?;
    for (index, expected) in [true, false, true].into_iter().enumerate() {
        let route = ready(scorer.route(&input, "Read a public file."))?;
        let backend = scorer.decider();
        if matches!(route, Route::AutoApprove(_)) != expected
            || backend.calls.load(std::sync::atomic::Ordering::Relaxed) != index + 1
        {
            return Ok(false);
        }
        if !expected
            && !matches!(route,
            Route::Escalate(Escalation::System1(System1Error::Transport { detail }))
                if detail.contains("intermittent failure"))
        {
            return Ok(false);
        }
    }
    Ok(scorer.decider().success.calls() == 2 && scorer.decider().failure.calls() == 1)
}

// Only dispatches to real MockBackends; no policy or protocol validation is implemented here.
struct AlternatingMocks {
    success: MockBackend,
    failure: MockBackend,
    calls: std::sync::atomic::AtomicUsize,
}

impl System1Decider for AlternatingMocks {
    async fn decide(&self, batch: &QuestionBatch) -> Result<AnswerBatch, System1Error> {
        let index = self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        match index {
            0 | 2 => self.success.decide(batch).await,
            1 => self.failure.decide(batch).await,
            _ => Err(System1Error::Protocol {
                reason: "three-call script exhausted".to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn alternating_transport_does_not_flap() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
