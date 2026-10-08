//! The evaluator seam. An evaluator applies a validated change-set
//! inside the experiment worktree and measures it — in production, by
//! running trainlab on the frozen dev split (and, when the change-set
//! calls for a training step, consuming a trainer receipt as evidence).
//! [`ScriptedEvaluator`] replays fixed outcomes so the orchestration
//! can be tested end to end with no models, no training, and no GPU.

use std::collections::VecDeque;

use crate::changeset::ChangeSet;
use crate::clock::Clock;
use crate::error::FailureClass;

/// The measured outcome of one experiment.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Evaluation {
    /// The metric: dev-split mean pass@1. Higher is better. Must be
    /// finite; the loop treats NaN/infinite as `metric_missing`.
    pub metric: f64,
    /// SHA-256 of the trainlab run receipt behind the metric, when
    /// the evaluation produced one. A malformed value (not 64 hex
    /// characters) is `metric_missing`, never a low score.
    pub trainlab_receipt_sha256: Option<String>,
    /// SHA-256 of the trainer receipt, when a training step fed this
    /// evaluation. Same evidence discipline as the trainlab hash.
    pub trainer_receipt_sha256: Option<String>,
    /// One-line description of what was measured.
    pub description: String,
}

impl Evaluation {
    /// A metric-only evaluation (scripted runs, dry configurations).
    #[must_use]
    pub fn metric_only(metric: f64, description: impl Into<String>) -> Self {
        Evaluation {
            metric,
            trainlab_receipt_sha256: None,
            trainer_receipt_sha256: None,
            description: description.into(),
        }
    }
}

/// An evaluation failure, classified for the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalError {
    /// Failure class. Only `evaluation_failed` and
    /// `evaluation_timeout` are honored as-is; the loop coerces any
    /// other class to `evaluation_failed` so an evaluator cannot mint
    /// taxonomy entries (e.g. it can never report `gate_violation`).
    pub class: FailureClass,
    /// Bounded human-readable detail.
    pub message: String,
}

impl EvalError {
    /// An `evaluation_failed` error with the given detail.
    #[must_use]
    pub fn failed(message: impl Into<String>) -> Self {
        EvalError {
            class: FailureClass::EvaluationFailed,
            message: message.into(),
        }
    }
}

/// Applies and measures one validated change-set.
pub trait Evaluator {
    /// Evaluate `change_set`. The loop measures wall clock around this
    /// call with `clock` and enforces the per-experiment budget on the
    /// result, whatever this returns.
    ///
    /// # Errors
    /// [`EvalError`] for every expected failure mode — crashes, sampler
    /// errors, trainer walls. Returning `Ok` with a non-finite metric
    /// is a contract violation the loop records as `metric_missing`.
    fn evaluate(
        &mut self,
        change_set: &ChangeSet,
        clock: &dyn Clock,
    ) -> Result<Evaluation, EvalError>;
}

/// One scripted evaluation outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptedOutcome {
    /// Return this evaluation.
    Evaluation(Evaluation),
    /// Fail with this class and message.
    Fail(FailureClass, String),
    /// Advance the clock (manual clocks only; simulates a slow run),
    /// then return a metric-only evaluation.
    AdvanceThenMetric {
        /// Milliseconds to advance before returning.
        advance_ms: u64,
        /// The metric to return.
        metric: f64,
    },
}

/// An evaluator that replays a fixed queue of outcomes. An empty queue
/// is an `evaluation_failed` error — a script that runs dry mid-loop is
/// a test bug, and it surfaces as one rather than as a silent metric.
#[derive(Debug, Default)]
pub struct ScriptedEvaluator {
    queue: VecDeque<ScriptedOutcome>,
}

impl ScriptedEvaluator {
    /// A scripted evaluator replaying `outcomes` in order.
    #[must_use]
    pub fn new(outcomes: Vec<ScriptedOutcome>) -> Self {
        ScriptedEvaluator {
            queue: outcomes.into(),
        }
    }

    /// Outcomes not yet consumed.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.queue.len()
    }
}

impl Evaluator for ScriptedEvaluator {
    fn evaluate(
        &mut self,
        _change_set: &ChangeSet,
        clock: &dyn Clock,
    ) -> Result<Evaluation, EvalError> {
        match self.queue.pop_front() {
            None => Err(EvalError::failed("scripted evaluator exhausted")),
            Some(ScriptedOutcome::Evaluation(evaluation)) => Ok(evaluation),
            Some(ScriptedOutcome::Fail(class, message)) => Err(EvalError { class, message }),
            Some(ScriptedOutcome::AdvanceThenMetric { advance_ms, metric }) => {
                clock.advance(advance_ms);
                Ok(Evaluation::metric_only(metric, "scripted slow run"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::ChangeKind;
    use crate::clock::ManualClock;

    fn change_set() -> ChangeSet {
        ChangeSet {
            id: "x".to_string(),
            kind: ChangeKind::TrainlabConfig,
            paths: Vec::new(),
            payload: "{}".to_string(),
            rationale: "test".to_string(),
        }
    }

    #[test]
    fn scripted_evaluator_replays_outcomes() {
        let clock = ManualClock::new();
        let mut evaluator = ScriptedEvaluator::new(vec![
            ScriptedOutcome::AdvanceThenMetric {
                advance_ms: 120,
                metric: 0.5,
            },
            ScriptedOutcome::Fail(FailureClass::EvaluationFailed, "boom".to_string()),
        ]);
        let first = evaluator
            .evaluate(&change_set(), &clock)
            .expect("first outcome");
        assert_eq!(first.metric, 0.5);
        assert_eq!(clock.now_ms(), 120);
        let err = evaluator
            .evaluate(&change_set(), &clock)
            .expect_err("second outcome");
        assert_eq!(err.class, FailureClass::EvaluationFailed);
        assert_eq!(evaluator.remaining(), 0);
    }
}
