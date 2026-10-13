//! The calibration tuple: the design's atomic unit, verbatim where
//! the design speaks and documented where it was silent (see the
//! crate-level docs).

use phlow_gauntlet::trace::GroundTruth as TraceGroundTruth;
use phlow_gauntlet::trace::is_held_out;
use phlow_system1::{Answer, Question, RichAnswer, STATE_BYTES_MAX};

use crate::CalibrationError;

/// Which of the design's three question kinds a tuple belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QuestionKind {
    /// Pick exactly one of the question's options.
    Choice,
    /// Yes or no.
    Noul,
    /// Rate on 0.0..=1.0 against the question's criteria.
    Score,
}

impl QuestionKind {
    /// The kind of a protocol question.
    pub fn of(question: &Question) -> QuestionKind {
        match question {
            Question::Choice { .. } => QuestionKind::Choice,
            Question::Noul { .. } => QuestionKind::Noul,
            Question::Score { .. } => QuestionKind::Score,
        }
    }

    /// The lowercase kind name used in reports and exports.
    pub fn label(self) -> &'static str {
        match self {
            QuestionKind::Choice => "choice",
            QuestionKind::Noul => "noul",
            QuestionKind::Score => "score",
        }
    }
}

/// The realized correct answer for one question, when a label exists.
///
/// Mirrors the design's `GroundTruth`; the kind must match the
/// question's kind or the tuple is rejected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GroundTruth {
    /// The index of the correct option of a Choice question.
    Choice {
        /// Index into the question's options.
        correct_index: usize,
    },
    /// Whether a Noul question's true answer is yes.
    Noul {
        /// True when the correct answer is yes.
        actually_yes: bool,
    },
    /// The true value of a Score question, on 0.0..=1.0.
    Score {
        /// The correct score.
        true_value: f64,
    },
}

impl GroundTruth {
    /// The question kind this truth labels.
    pub fn kind(self) -> QuestionKind {
        match self {
            GroundTruth::Choice { .. } => QuestionKind::Choice,
            GroundTruth::Noul { .. } => QuestionKind::Noul,
            GroundTruth::Score { .. } => QuestionKind::Score,
        }
    }

    /// Convert the gauntlet trace record's ground truth (same shape,
    /// defined by the capture that writes the corpus).
    pub(crate) fn from_trace(truth: TraceGroundTruth) -> GroundTruth {
        match truth {
            TraceGroundTruth::Choice { correct_index } => GroundTruth::Choice { correct_index },
            TraceGroundTruth::Noul { actually_yes } => GroundTruth::Noul { actually_yes },
            TraceGroundTruth::Score { true_value } => GroundTruth::Score { true_value },
        }
    }
}

/// Where a tuple came from (the design's `TupleSource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TupleSource {
    /// From Matt's approval history. Ground truth = his decision.
    Approval {
        /// Id of the approval record.
        approval_id: String,
    },
    /// From production feedback. Ground truth may be pending.
    Feedback {
        /// Id of the session the feedback belongs to.
        session_id: String,
    },
    /// From a gauntlet task run.
    Gauntlet {
        /// Id of the gauntlet task that made the decision.
        task_id: String,
    },
}

impl TupleSource {
    /// Dataset weight by source (design Phase B): gauntlet 1.0,
    /// approval 2.0 (Matt's labels are gold), feedback 0.5 (weak).
    pub fn weight(&self) -> f64 {
        match self {
            TupleSource::Approval { .. } => 2.0,
            TupleSource::Feedback { .. } => 0.5,
            TupleSource::Gauntlet { .. } => 1.0,
        }
    }

    /// The gauntlet task id when the source is a gauntlet task.
    pub fn gauntlet_task_id(&self) -> Option<&str> {
        match self {
            TupleSource::Gauntlet { task_id } => Some(task_id),
            _ => None,
        }
    }
}

/// One scored decision point: the design's `CalibrationTuple`.
///
/// `predicted_dist` is an `Option` where the design shows a bare
/// `Vec<f64>`: scalar backends genuinely expose no distribution for
/// Choice and Score answers, and the trace corpus records `null`
/// there. Collapsing that absence into an invented vector would
/// fabricate the very signal this crate exists to measure.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationTuple {
    /// The state string sent to the decision model.
    pub state: String,
    /// The question that was asked.
    pub question: Question,
    /// What the model predicted.
    pub predicted: Answer,
    /// The full predicted distribution in option order, when the
    /// backend genuinely exposed one.
    pub predicted_dist: Option<Vec<f64>>,
    /// The realized label, or `None` while the label is pending.
    pub ground_truth: Option<GroundTruth>,
    /// Where this tuple came from.
    pub source: TupleSource,
    /// When the decision was made, epoch milliseconds (the design's
    /// `decided_at`, stored without a chrono dependency — the same
    /// lossless choice the trace capture made).
    pub decided_at_unix_ms: u64,
    /// Whether this tuple sits in the held-out validation reserve.
    /// Enforced by task ID for gauntlet tuples (see
    /// [`CalibrationTuple::validate`]).
    pub held_out: bool,
}

impl CalibrationTuple {
    /// The kind of this tuple's question.
    pub fn kind(&self) -> QuestionKind {
        QuestionKind::of(&self.question)
    }

    /// Check the tuple against the calibration contract: a state
    /// within the protocol bound, an answer of the question's kind
    /// with in-range scalars, a ground truth of the question's kind
    /// when present, a genuine distribution when present (checked by
    /// the real [`RichAnswer::validate`]), and — for gauntlet
    /// tuples — a held-out stamp that agrees with the task-ID split.
    pub fn validate(&self) -> Result<(), CalibrationError> {
        let invalid = |reason: &str| CalibrationError::Validation {
            reason: reason.to_string(),
        };
        if self.state.len() > STATE_BYTES_MAX {
            return Err(invalid("state exceeds STATE_BYTES_MAX"));
        }
        let kind_matches = matches!(
            (&self.question, self.predicted),
            (Question::Choice { .. }, Answer::Choice { .. })
                | (Question::Noul { .. }, Answer::Noul { .. })
                | (Question::Score { .. }, Answer::Score { .. })
        );
        if !kind_matches {
            return Err(invalid("answer kind does not match question kind"));
        }
        let confidence = self.predicted.confidence();
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(invalid("answer confidence is not a finite number in 0..=1"));
        }
        if let Answer::Score { value, .. } = self.predicted
            && (!value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(invalid("score value is not a finite number in 0..=1"));
        }
        if let Some(truth) = self.ground_truth {
            if truth.kind() != self.kind() {
                return Err(invalid("ground truth kind does not match question kind"));
            }
            if let GroundTruth::Score { true_value } = truth
                && (!true_value.is_finite() || !(0.0..=1.0).contains(&true_value))
            {
                return Err(invalid("true score is not a finite number in 0..=1"));
            }
            if let (Question::Choice { options, .. }, GroundTruth::Choice { correct_index }) =
                (&self.question, truth)
                && correct_index >= options.len()
            {
                return Err(invalid("correct index is out of range for the options"));
            }
        }
        if let Some(dist) = &self.predicted_dist {
            let rich = RichAnswer {
                answer: self.predicted,
                distribution: dist.clone(),
            };
            rich.validate(&self.question)
                .map_err(|error| CalibrationError::Validation {
                    reason: error.to_string(),
                })?;
        }
        if let Some(task_id) = self.source.gauntlet_task_id()
            && self.held_out != is_held_out(task_id)
        {
            return Err(invalid("held-out stamp disagrees with the task-ID split"));
        }
        Ok(())
    }

    /// The (confidence in the predicted answer, prediction was
    /// right) pair the calibration curves and the design's Brier
    /// formula consume, or `None` while the label is pending.
    ///
    /// Score correctness is exact equality of value and true value
    /// (see the crate-level docs for why no tolerance is invented).
    pub fn scored_outcome(&self) -> Option<(f64, bool)> {
        let truth = self.ground_truth?;
        let (confidence, correct) = match (self.predicted, truth) {
            (
                Answer::Choice {
                    selected,
                    probability,
                },
                GroundTruth::Choice { correct_index },
            ) => (probability, selected == correct_index),
            (Answer::Noul { yes, probability }, GroundTruth::Noul { actually_yes }) => {
                let confidence = if yes { probability } else { 1.0 - probability };
                (confidence, yes == actually_yes)
            }
            (Answer::Score { value, confidence }, GroundTruth::Score { true_value }) => {
                (confidence, value == true_value)
            }
            _ => return None,
        };
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return None;
        }
        Some((confidence, correct))
    }

    /// The full predicted distribution, when one genuinely exists:
    /// the recorded distribution, or — for Noul — the Bernoulli
    /// completion `[1 - p, p]`, which is the protocol's own
    /// construction of the full distribution from its scalar (the
    /// same construction `RichAnswer::validate` defines). Choice
    /// and Score answers without a recorded distribution have none.
    pub fn full_distribution(&self) -> Option<Vec<f64>> {
        if let Some(dist) = &self.predicted_dist {
            return match self.kind() {
                QuestionKind::Score => None,
                _ => Some(dist.clone()),
            };
        }
        match self.predicted {
            Answer::Noul { probability, .. } => Some(vec![1.0 - probability, probability]),
            _ => None,
        }
    }

    /// The one-hot ground-truth distribution aligned with
    /// [`CalibrationTuple::full_distribution`], or `None` when the
    /// tuple is unlabeled, is a Score (a scalar truth cannot be
    /// aligned one-hot over distribution levels), or no full
    /// distribution exists.
    pub fn truth_distribution(&self) -> Option<Vec<f64>> {
        let dist = self.full_distribution()?;
        match (self.question.clone(), self.ground_truth?) {
            (Question::Choice { options, .. }, GroundTruth::Choice { correct_index }) => {
                let mut one_hot = vec![0.0; options.len()];
                *one_hot.get_mut(correct_index)? = 1.0;
                Some(one_hot)
            }
            (_, GroundTruth::Noul { actually_yes }) => Some(if actually_yes {
                vec![0.0, 1.0]
            } else {
                vec![1.0, 0.0]
            }),
            _ => None,
        }
        .filter(|one_hot| one_hot.len() == dist.len())
    }
}
