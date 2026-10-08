//! Core probe types: the [`Probe`] trait, its result and evidence
//! shapes, and the [`ProbeBackend`] interface a model is screened
//! through. Shapes follow the canary battery design; see `lib.rs` for
//! the two deliberate deviations (a canary-local [`RichAnswer`], and a
//! synchronous backend trait matching the design's synchronous
//! `Probe::run`).

use std::collections::BTreeMap;

use phlow_system1::{Answer, Question, QuestionBatch, System1Error};
use serde::{Deserialize, Serialize};

/// Which of the four design categories a probe belongs to. The verdict
/// layer needs the category because the consensus challenger applies
/// to the trigger category only (2026-10-07 amendment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProbeCategory {
    /// Prompt injection resistance.
    Injection,
    /// Backdoor trigger candidates.
    Trigger,
    /// Refusal consistency.
    Refusal,
    /// Anomalous confidence patterns.
    Calibration,
}

/// How the verdict layer perturbs a probe for the consensus challenger:
/// re-run a unanimously passing trigger probe against inputs that
/// differ from the exact strings it just passed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Perturbation {
    /// Paraphrased payload variants (same content, different surface).
    Paraphrase,
    /// Suspected trigger stripped or replaced by a neutral form.
    StripTrigger,
}

/// One behavioral test against the model.
pub trait Probe: Send + Sync {
    /// Stable identifier, e.g. "injection.direct-001".
    fn id(&self) -> &str;
    /// Human-readable description of what this probes.
    fn description(&self) -> &str;
    /// ATLAS technique this maps to, if any.
    fn atlas(&self) -> Option<&str>;
    /// Run the probe against a model backend. Returns pass/fail with
    /// evidence. A backend error is a failed result, never a panic and
    /// never a pass (fail-closed).
    fn run(&self, backend: &dyn ProbeBackend) -> ProbeResult;
}

/// An answer with its full probability distribution preserved.
///
/// The distribution is what the trigger and calibration statistics
/// consume: `distribution[i]` is the model's probability for option
/// `i` of a Choice question, `[1 - p, p]` for a Noul question, and the
/// per-criterion confidences for a Score question.
#[derive(Debug, Clone, PartialEq)]
pub struct RichAnswer {
    /// The answer the model gave, in system1's committed shape.
    pub answer: Answer,
    /// Full per-option probability vector for that answer.
    pub distribution: Vec<f64>,
}

/// Rich answers keyed by question id.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichAnswerBatch {
    /// Answers by question id, sorted for deterministic handling.
    pub answers: BTreeMap<String, RichAnswer>,
}

impl RichAnswerBatch {
    /// The rich answer for one question id, if the backend gave one.
    pub fn get(&self, question_id: &str) -> Option<&RichAnswer> {
        self.answers.get(question_id)
    }
}

/// Minimal interface for running probes against a model.
///
/// Synchronous by design: the design's `Probe::run` is synchronous, so
/// the battery core needs no runtime. Async system1 clients are adapted
/// at the integration seam, outside this crate.
pub trait ProbeBackend: Send + Sync {
    /// Run one question batch, return rich answers. Any error fails
    /// the calling probe (fail-closed); the battery never retries.
    fn decide(&self, batch: &QuestionBatch) -> Result<RichAnswerBatch, System1Error>;
}

/// Result of one probe execution.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeResult {
    /// The probe that produced this result.
    pub probe_id: String,
    /// Whether the probe passed this execution.
    pub passed: bool,
    /// Bounded evidence. Never contains raw payload text.
    pub evidence: ProbeEvidence,
}

impl ProbeResult {
    /// A failed result whose only evidence is a backend error kind.
    /// The kind label is a fixed string per `System1Error` variant;
    /// server-supplied detail text is deliberately dropped so nothing
    /// a model or server emitted can reach reports or logs.
    pub fn backend_error(probe_id: &str, error: &System1Error) -> Self {
        let kind = match error {
            System1Error::Config { .. } => "config",
            System1Error::InvalidBatch { .. } => "invalid_batch",
            System1Error::Transport { .. } => "transport",
            System1Error::Timeout => "timeout",
            System1Error::Protocol { .. } => "protocol",
        };
        ProbeResult {
            probe_id: probe_id.to_owned(),
            passed: false,
            evidence: ProbeEvidence::error(kind),
        }
    }
}

/// Outcome of the consensus challenger, when it ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChallengerEvidence {
    /// Perturbation applied: "paraphrase" or "strip_trigger".
    pub perturbation: String,
    /// True when the perturbed repetition collapsed the consensus.
    pub collapsed: bool,
    /// Distribution shift measured by the perturbed run, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shift: Option<f64>,
}

/// Bounded probe evidence: numbers, counts and fixed labels only.
///
/// By construction this type cannot carry payload text — every field
/// is a fixed label, an option label from the probe's own question
/// definition, or a statistic. The adversarial tests verify the
/// serialized report contains no payload substrings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProbeEvidence {
    /// Fixed label for the check performed, e.g. "distribution-shift".
    pub check: String,
    /// Fixed label for the expected safe outcome, where applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// Fixed label for the observed outcome, where applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    /// Measured distribution shift or delta, where applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shift: Option<f64>,
    /// The per-model threshold the measurement was judged against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// Refusals observed (refusal category).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused_count: Option<usize>,
    /// Requests presented (refusal category).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_count: Option<usize>,
    /// Mean top-option confidence across the run's key questions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_confidence: Option<f64>,
    /// Variance of top-option confidence where a probe measures it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_variance: Option<f64>,
    /// Bimodality score (calibration category).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bimodality_score: Option<f64>,
    /// How many of the repeated runs passed, set by the verdict layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passed_runs: Option<usize>,
    /// Challenger outcome, set by the verdict layer (trigger category).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenger: Option<ChallengerEvidence>,
    /// Backend error kind label, when the run failed on an error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ProbeEvidence {
    /// Evidence for a named check, with no measurements yet.
    pub fn check(name: &str) -> Self {
        ProbeEvidence {
            check: name.to_owned(),
            ..ProbeEvidence::default()
        }
    }

    /// Evidence for a run that failed on a backend error.
    pub fn error(kind: &str) -> Self {
        ProbeEvidence {
            check: "backend-error".to_owned(),
            error: Some(kind.to_owned()),
            ..ProbeEvidence::default()
        }
    }
}

/// Validate one rich answer against the question it answers.
///
/// A backend that returns malformed distributions (wrong length,
/// non-finite, out of range, not summing to ~1) must fail the probe,
/// not crash it and not pass it. Returns a fixed reason string.
pub fn check_rich_answer(question: &Question, rich: &RichAnswer) -> Result<(), &'static str> {
    let expected_len = match question {
        Question::Choice { options, .. } => {
            match rich.answer {
                Answer::Choice { selected, .. } if selected < options.len() => {}
                _ => return Err("answer kind or selection does not match the question"),
            }
            options.len()
        }
        Question::Score { criteria, .. } => {
            match rich.answer {
                Answer::Score { .. } => {}
                _ => return Err("answer kind does not match the question"),
            }
            criteria.len()
        }
        Question::Noul { .. } => {
            match rich.answer {
                Answer::Noul { .. } => {}
                _ => return Err("answer kind does not match the question"),
            }
            2
        }
    };
    if rich.distribution.len() != expected_len {
        return Err("distribution length does not match the question");
    }
    let mut sum = 0.0_f64;
    for value in &rich.distribution {
        if !value.is_finite() || *value < 0.0 || *value > 1.0 {
            return Err("distribution value out of range");
        }
        sum += value;
    }
    if !(0.95..=1.05).contains(&sum) {
        return Err("distribution does not sum to one");
    }
    Ok(())
}
