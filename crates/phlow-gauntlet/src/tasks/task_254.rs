//! task-254: simple negation in state (validation).
//!
//! Script a deterministic Choice answer; preserve the exact state, instructions, and option order.
//! This characterizes protocol fidelity, not the semantic accuracy of a live model.

use super::task_254::support::{check_choice, choice_batch, decide, scripted_choice};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-254";
/// Concern exercised against the scripted System 1 backend.
pub const NAME: &str = "simple negation in state";
/// Offline Rust driver; no live model or HTTP service.
pub const KIND: TaskKind = TaskKind::Rust;

/// Run the one validation scenario, retaining errors as failed evidence.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, std::iter::once(("simple_negation_in_state", case())))
}

fn case() -> Result<bool, String> {
    let batch = choice_batch(
        "Do not cancel the pending appointment.",
        "Select the requested action.",
        &["keep appointment", "cancel appointment"],
    );
    let backend = scripted_choice(0, 0.99);
    let answer = decide(&backend, &batch)?;
    check_choice(&backend, &batch, &answer, 0, 0.99)
}

/// Shared fixtures call the real public protocol and risk APIs. They contain no policy logic.
/// MockBackend is intentionally untrusted; answer validation is an explicit production call.
pub(super) mod support {
    pub(crate) use phlow_approval::Risk;
    use phlow_approval::{Decision, Request, Verdict};
    pub(crate) use phlow_system1::{
        Answer, AnswerBatch, Escalation, IRREVERSIBLE_ID, MockBackend, Question, QuestionBatch,
        RiskScorer, Route, System1Decider, System1Error,
    };
    use phlow_system1::{CONSISTENT_ID, FORBIDDEN_ID, REVERSIBLE_ID, RISK_ID};
    pub(crate) use std::collections::BTreeMap;

    // Production constants are explicitly documented as UNVALIDATED PLACEHOLDERs.
    // These tests characterize boundaries, not calibration on a held-out workload.
    pub(crate) const RISK_THRESHOLD: f64 = phlow_system1::RISK_MAX;
    pub(crate) const MIN_CONFIDENCE: f64 = phlow_system1::CONFIDENCE_MIN;
    /// Larger than roundoff, small relative to either threshold.
    pub(crate) const EPSILON: f64 = 1e-9;

    pub(crate) fn error_text(error: impl std::fmt::Display) -> String {
        error.to_string()
    }

    pub(crate) fn choice_batch(state: &str, instructions: &str, options: &[&str]) -> QuestionBatch {
        QuestionBatch {
            state: state.to_owned(),
            questions: BTreeMap::from([(
                "q".to_owned(),
                Question::Choice {
                    instructions: instructions.to_owned(),
                    options: options.iter().map(|option| (*option).to_owned()).collect(),
                },
            )]),
        }
    }

    pub(crate) fn noul_batch(state: &str, instructions: &str) -> QuestionBatch {
        QuestionBatch {
            state: state.to_owned(),
            questions: BTreeMap::from([(
                "q".to_owned(),
                Question::Noul {
                    instructions: instructions.to_owned(),
                },
            )]),
        }
    }

    pub(crate) fn score_batch(state: &str, instructions: &str, criteria: &[&str]) -> QuestionBatch {
        QuestionBatch {
            state: state.to_owned(),
            questions: BTreeMap::from([(
                "q".to_owned(),
                Question::Score {
                    instructions: instructions.to_owned(),
                    criteria: criteria
                        .iter()
                        .map(|criterion| (*criterion).to_owned())
                        .collect(),
                },
            )]),
        }
    }

    pub(crate) fn answer_batch<const COUNT: usize>(
        answers: [(&str, Answer); COUNT],
    ) -> AnswerBatch {
        AnswerBatch {
            answers: answers
                .into_iter()
                .map(|(id, answer)| (id.to_owned(), answer))
                .collect(),
        }
    }

    pub(crate) fn mock_answers(answers: AnswerBatch) -> MockBackend {
        answers
            .answers
            .into_iter()
            .fold(MockBackend::new(), |backend, (id, answer)| {
                backend.with_answer(&id, answer)
            })
    }

    pub(crate) fn scripted_choice(selected: usize, probability: f64) -> MockBackend {
        MockBackend::new().with_answer(
            "q",
            Answer::Choice {
                selected,
                probability,
            },
        )
    }

    pub(crate) fn scripted_noul(yes: bool, probability: f64) -> MockBackend {
        MockBackend::new().with_answer("q", Answer::Noul { yes, probability })
    }

    pub(crate) fn scripted_score(value: f64, confidence: f64) -> MockBackend {
        MockBackend::new().with_answer("q", Answer::Score { value, confidence })
    }

    pub(crate) fn risk_backend(value: f64, confidence: f64, yes: bool) -> MockBackend {
        mock_answers(answer_batch([
            (RISK_ID, Answer::Score { value, confidence }),
            (
                REVERSIBLE_ID,
                Answer::Noul {
                    yes,
                    probability: 0.99,
                },
            ),
            (
                IRREVERSIBLE_ID,
                Answer::Noul {
                    yes: !yes,
                    probability: 0.99,
                },
            ),
            (
                CONSISTENT_ID,
                Answer::Noul {
                    yes: true,
                    probability: 0.99,
                },
            ),
            (
                FORBIDDEN_ID,
                Answer::Noul {
                    yes: false,
                    probability: 0.99,
                },
            ),
        ]))
    }

    /// Only immediately ready in-memory mock futures are permitted in this bounded fixture.
    pub(crate) fn ready<F: std::future::Future>(future: F) -> Result<F::Output, String> {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(output) => Ok(output),
            std::task::Poll::Pending => {
                Err("scripted backend unexpectedly yielded Pending".to_owned())
            }
        }
    }

    pub(crate) fn decide_result(
        backend: &MockBackend,
        batch: &QuestionBatch,
    ) -> Result<Result<AnswerBatch, System1Error>, String> {
        // Exercise public validation, never reimplement it. The untrusted mock returns raw
        // answers; consumers must validate exactly as the real RiskScorer does.
        let answers = ready(backend.decide(batch))?;
        Ok(answers.and_then(|answers| {
            answers.validate(batch)?;
            Ok(answers)
        }))
    }

    pub(crate) fn decide(
        backend: &MockBackend,
        batch: &QuestionBatch,
    ) -> Result<AnswerBatch, String> {
        decide_result(backend, batch)?.map_err(error_text)
    }

    /// A borrowed forwarding fixture so assertions can inspect the same MockBackend later.
    struct BorrowedMock<'a>(&'a MockBackend);
    impl System1Decider for BorrowedMock<'_> {
        async fn decide(&self, batch: &QuestionBatch) -> Result<AnswerBatch, System1Error> {
            self.0.decide(batch).await
        }
    }

    pub(crate) fn approval_decision(risk: Risk) -> Result<Decision, String> {
        let request = Request::from_json(&serde_json::json!({
            "tool": "fs.write", "risk": risk.as_str(), "paths": ["/work/test.txt"],
        }))
        .map_err(error_text)?;
        Ok(Decision {
            verdict: Verdict::Approval,
            reason: "test requires approval",
            scope: request.scope().clone(),
        })
    }

    pub(crate) fn assess(backend: &MockBackend, state: &str, risk: Risk) -> Result<Route, String> {
        let scorer = RiskScorer::new(BorrowedMock(backend));
        ready(scorer.route(&approval_decision(risk)?, state))
    }

    pub(crate) fn check_choice(
        backend: &MockBackend,
        batch: &QuestionBatch,
        answer: &AnswerBatch,
        selected: usize,
        probability: f64,
    ) -> Result<bool, String> {
        let wire = answer.to_wire().map_err(error_text)?;
        let decoded = AnswerBatch::from_wire(batch, &wire).map_err(error_text)?;
        Ok(decoded == *answer
            && answer.answers.len() == 1
            && matches!(answer.answers.get("q"), Some(Answer::Choice { selected: actual, probability: p })
                if *actual == selected && (*p - probability).abs() < EPSILON)
            && backend.calls() == 1
            && backend.last_batch().as_ref() == Some(batch))
    }

    pub(crate) fn check_noul(
        backend: &MockBackend,
        batch: &QuestionBatch,
        answer: &AnswerBatch,
        yes: bool,
        probability: f64,
    ) -> Result<bool, String> {
        let wire = answer.to_wire().map_err(error_text)?;
        let decoded = AnswerBatch::from_wire(batch, &wire).map_err(error_text)?;
        Ok(decoded == *answer
            && answer.answers.len() == 1
            && matches!(answer.answers.get("q"), Some(Answer::Noul { yes: actual, probability: p })
                if *actual == yes && (*p - probability).abs() < EPSILON)
            && backend.calls() == 1
            && backend.last_batch().as_ref() == Some(batch))
    }

    pub(crate) fn check_score(
        backend: &MockBackend,
        batch: &QuestionBatch,
        answer: &AnswerBatch,
        value: f64,
        confidence: f64,
    ) -> Result<bool, String> {
        let wire = answer.to_wire().map_err(error_text)?;
        let decoded = AnswerBatch::from_wire(batch, &wire).map_err(error_text)?;
        Ok(decoded == *answer
            && answer.answers.len() == 1
            && matches!(answer.answers.get("q"), Some(Answer::Score { value: actual, confidence: c })
                if (*actual - value).abs() < EPSILON && (*c - confidence).abs() < EPSILON)
            && backend.calls() == 1
            && backend.last_batch().as_ref() == Some(batch))
    }

    pub(crate) fn score_value(answer: &AnswerBatch) -> Result<f64, String> {
        match answer.answers.get("q") {
            Some(Answer::Score { value, .. }) if value.is_finite() => Ok(*value),
            _ => Err("missing, nonfinite, or wrong-kind score answer for q".to_owned()),
        }
    }

    pub(crate) fn check_decision(
        backend: &MockBackend,
        decision: &Route,
        expected_auto_approve: bool,
    ) -> Result<bool, String> {
        let expected = match decision {
            Route::AutoApprove(_) => expected_auto_approve && backend.calls() == 1,
            Route::Escalate(reason) => !expected_auto_approve && !reason.to_string().is_empty(),
        };
        Ok(expected && backend.calls() <= 1)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn simple_negation_in_state() {
        let result = super::case();
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
