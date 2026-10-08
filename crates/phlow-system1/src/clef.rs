//! Clef-compatible backend for the System 1 decision protocol.
//!
//! Cloudflare's Clef models speak the Jev/SystemOne wire protocol but with
//! a different response shape than `laya-serve`. This module translates
//! Clef's responses into phlow's internal [`Answer`] types.
//!
//! ## Wire format differences
//!
//! Request: identical. `{"model": m, "state": s, "questions": {...}}`.
//!
//! Response envelope: Clef wraps answers in
//! `{"model": m, "answers": {qid: {...}}, "usage": {...}}`.
//! Phlow's [`AnswerBatch`] expects the bare answers object.
//!
//! Per-answer shapes:
//! - Noul: Clef `{"type": "noul", "noul": p}` → phlow `Noul { yes: p > 0.5, probability: p }`
//! - Choice: Clef `{"type": "choice", "choice": "<id>", "confidence": c, "probabilities": {...}}`
//!   → phlow `Choice { selected: <index of id>, probability: probs[id] }`
//! - Score: Clef `{"type": "score", "score": v, "confidence": c, ...}`
//!   → phlow `Score { value: v, confidence: c }`
//!
//! ## Rich answers
//!
//! [`RichAnswer`] preserves the full probability distribution from Clef's
//! response, which [`Answer`] collapses to a single value. The calibration
//! harness (`phlow-calibration`) needs the distribution for Brier scores.
//! [`crate::RiskScorer`] continues to use the simple [`Answer`].

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::System1Error;
use crate::protocol::{Answer, AnswerBatch, Question, QuestionBatch};

// Maximum answers in one Clef response (matches QUESTIONS_MAX).
// (Reuses crate::protocol::QUESTIONS_MAX via validation.)

/// An answer with its full probability distribution preserved.
///
/// `answer` is the translated phlow [`Answer`] for [`crate::RiskScorer`].
/// `distribution` is the full per-option probability vector from Clef,
/// in option order, for the calibration harness. For Noul questions this
/// is `[P(no), P(yes)]`. For Score questions this is the per-level
/// probabilities from Clef's `probabilities` dict.
#[derive(Debug, Clone, PartialEq)]
pub struct RichAnswer {
    pub answer: Answer,
    pub distribution: Vec<f64>,
}

/// A batch of rich answers, keyed by question id.
#[derive(Debug, Clone, PartialEq)]
pub struct RichAnswerBatch {
    pub answers: BTreeMap<String, RichAnswer>,
}

/// How far a distribution's sum may drift from 1.0 and still count as
/// a distribution. Named and stated because every consumer of a rich
/// answer relies on it: measured probabilities arrive as decimal
/// text, so an exact-1.0 demand would reject honest data, while a
/// loose band would let fabricated vectors through.
pub const DISTRIBUTION_SUM_TOLERANCE: f64 = 1e-6;

impl RichAnswer {
    /// Validate this rich answer against the question it answers:
    /// the answer kind matches the question kind, the distribution
    /// has exactly one entry per option (Noul: `[1 - p, p]` in
    /// `[P(no), P(yes)]` order; Choice: option order; Score: level
    /// order), every entry is finite and in `0.0..=1.0`, and the
    /// entries sum to 1.0 within [`DISTRIBUTION_SUM_TOLERANCE`].
    pub fn validate(&self, question: &Question) -> Result<(), System1Error> {
        let protocol = |msg: &str| System1Error::protocol(msg);
        let expected_len = match (question, &self.answer) {
            (Question::Noul { .. }, Answer::Noul { .. }) => 2,
            (Question::Choice { options, .. }, Answer::Choice { .. }) => options.len(),
            (Question::Score { criteria, .. }, Answer::Score { .. }) => criteria.len(),
            _ => return Err(protocol("answer kind does not match question kind")),
        };
        if self.distribution.len() != expected_len {
            return Err(protocol("distribution length does not match the question"));
        }
        let mut sum = 0.0_f64;
        for value in &self.distribution {
            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                return Err(protocol("distribution value is not finite in 0..=1"));
            }
            sum += value;
        }
        if (sum - 1.0).abs() > DISTRIBUTION_SUM_TOLERANCE {
            return Err(protocol("distribution does not sum to one"));
        }
        Ok(())
    }
}

impl RichAnswerBatch {
    /// The simple answers, for [`crate::RiskScorer`].
    pub fn to_answer_batch(&self) -> AnswerBatch {
        AnswerBatch {
            answers: self
                .answers
                .iter()
                .map(|(id, rich)| (id.clone(), rich.answer))
                .collect(),
        }
    }

    /// The rich answer for one question id, if the backend gave one.
    pub fn get(&self, question_id: &str) -> Option<&RichAnswer> {
        self.answers.get(question_id)
    }

    /// Validate every rich answer against `batch`: the scalar checks
    /// of [`AnswerBatch::validate`] plus the distribution checks of
    /// [`RichAnswer::validate`]. A backend producing rich answers
    /// runs this before any consumer trusts a distribution.
    pub fn validate(&self, batch: &QuestionBatch) -> Result<(), System1Error> {
        self.to_answer_batch().validate(batch)?;
        for (id, question) in &batch.questions {
            let rich = self
                .answers
                .get(id)
                .ok_or_else(|| System1Error::protocol("missing rich answer"))?;
            rich.validate(question)?;
        }
        Ok(())
    }
}

// Clef wire shapes. Closed: unknown fields reject.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // model/usage are part of the wire format; answers is what we use.
struct ClefEnvelope {
    model: String,
    answers: BTreeMap<String, serde_json::Value>,
    usage: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClefNoul {
    #[serde(rename = "type")]
    kind: String,
    noul: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // confidence duplicates probabilities[choice]; kept for wire compat.
struct ClefChoice {
    #[serde(rename = "type")]
    kind: String,
    choice: String,
    confidence: f64,
    probabilities: BTreeMap<String, f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // legend maps levels to labels; kept for wire compat.
struct ClefScore {
    #[serde(rename = "type")]
    kind: String,
    score: f64,
    confidence: f64,
    legend: BTreeMap<String, String>,
    probabilities: BTreeMap<String, f64>,
}

/// Decode one Clef answer against its question.
///
/// Returns the translated [`RichAnswer`], or a protocol error. Never
/// coerces: unknown option ids, missing probabilities, and out-of-range
/// numbers are errors.
fn decode_clef_answer(
    question: &Question,
    value: serde_json::Value,
) -> Result<RichAnswer, System1Error> {
    let protocol = |msg: &str| System1Error::protocol(msg);
    match question {
        Question::Noul { .. } => {
            let wire: ClefNoul =
                serde_json::from_value(value).map_err(|e| protocol(&e.to_string()))?;
            if wire.kind != "noul" {
                return Err(protocol("noul answer has wrong type tag"));
            }
            if !(0.0..=1.0).contains(&wire.noul) || !wire.noul.is_finite() {
                return Err(protocol("noul probability out of range"));
            }
            Ok(RichAnswer {
                answer: Answer::Noul {
                    yes: wire.noul > 0.5,
                    probability: wire.noul,
                },
                distribution: vec![1.0 - wire.noul, wire.noul],
            })
        }
        Question::Choice { options, .. } => {
            let wire: ClefChoice =
                serde_json::from_value(value).map_err(|e| protocol(&e.to_string()))?;
            if wire.kind != "choice" {
                return Err(protocol("choice answer has wrong type tag"));
            }
            let selected = options
                .iter()
                .position(|opt| *opt == wire.choice)
                .ok_or_else(|| protocol("choice answer references unknown option id"))?;
            let probability = wire
                .probabilities
                .get(&wire.choice)
                .copied()
                .ok_or_else(|| protocol("choice probabilities missing selected option"))?;
            if !(0.0..=1.0).contains(&probability) || !probability.is_finite() {
                return Err(protocol("choice probability out of range"));
            }
            // Distribution in option order; missing options are protocol errors.
            let mut distribution = Vec::with_capacity(options.len());
            for opt in options {
                let p = wire
                    .probabilities
                    .get(opt)
                    .copied()
                    .ok_or_else(|| protocol("choice probabilities missing option"))?;
                if !(0.0..=1.0).contains(&p) || !p.is_finite() {
                    return Err(protocol("choice distribution probability out of range"));
                }
                distribution.push(p);
            }
            Ok(RichAnswer {
                answer: Answer::Choice {
                    selected,
                    probability,
                },
                distribution,
            })
        }
        Question::Score { criteria, .. } => {
            let wire: ClefScore =
                serde_json::from_value(value).map_err(|e| protocol(&e.to_string()))?;
            if wire.kind != "score" {
                return Err(protocol("score answer has wrong type tag"));
            }
            if !(0.0..=1.0).contains(&wire.score)
                || !wire.score.is_finite()
                || !(0.0..=1.0).contains(&wire.confidence)
                || !wire.confidence.is_finite()
            {
                return Err(protocol("score value or confidence out of range"));
            }
            // Distribution in level order (0..criteria.len()).
            let mut distribution = Vec::with_capacity(criteria.len());
            for i in 0..criteria.len() {
                let level = i.to_string();
                let p = wire
                    .probabilities
                    .get(&level)
                    .copied()
                    .ok_or_else(|| protocol("score probabilities missing level"))?;
                if !(0.0..=1.0).contains(&p) || !p.is_finite() {
                    return Err(protocol("score distribution probability out of range"));
                }
                distribution.push(p);
            }
            Ok(RichAnswer {
                answer: Answer::Score {
                    value: wire.score,
                    confidence: wire.confidence,
                },
                distribution,
            })
        }
    }
}

impl RichAnswerBatch {
    /// Decode a Clef response body for `batch`.
    ///
    /// Unwraps the `{"model", "answers", "usage"}` envelope, then decodes
    /// each answer with [`decode_clef_answer`]. Rejects the same malformed
    /// inputs as [`AnswerBatch::from_wire`], plus Clef-specific mismatches
    /// (unknown option ids, missing probabilities).
    pub fn from_clef_wire(batch: &QuestionBatch, body: &[u8]) -> Result<Self, System1Error> {
        use crate::protocol::RESPONSE_BYTES_MAX;
        if body.len() > RESPONSE_BYTES_MAX {
            return Err(System1Error::protocol(
                "response exceeds RESPONSE_BYTES_MAX",
            ));
        }
        let envelope: ClefEnvelope =
            serde_json::from_slice(body).map_err(|e| System1Error::protocol(&e.to_string()))?;
        let mut answers = BTreeMap::new();
        for (id, value) in envelope.answers {
            let Some(question) = batch.questions.get(&id) else {
                return Err(System1Error::protocol("answer for an unasked question id"));
            };
            answers.insert(id, decode_clef_answer(question, value)?);
        }
        let rich = RichAnswerBatch { answers };
        // Validate answer count matches question count, and every
        // distribution is a real distribution (fail-closed).
        rich.validate(batch)?;
        Ok(rich)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::QuestionBatch;
    use std::collections::BTreeMap;

    fn noul_batch() -> QuestionBatch {
        let mut questions = BTreeMap::new();
        questions.insert(
            "urgent".to_string(),
            Question::Noul {
                instructions: "Is it urgent?".to_string(),
            },
        );
        QuestionBatch {
            state: "test".to_string(),
            questions,
        }
    }

    fn choice_batch() -> QuestionBatch {
        let mut questions = BTreeMap::new();
        questions.insert(
            "team".to_string(),
            Question::Choice {
                instructions: "Which team?".to_string(),
                options: vec![
                    "billing".to_string(),
                    "technical".to_string(),
                    "sales".to_string(),
                ],
            },
        );
        QuestionBatch {
            state: "test".to_string(),
            questions,
        }
    }

    // --- Validation: correct translations ---

    #[test]
    fn decodes_clef_noul() {
        let batch = noul_batch();
        let body = br#"{"model": "clef-flash", "answers": {"urgent": {"type": "noul", "noul": 0.85}}, "usage": {"input_tokens": 10, "output_tokens": 0}}"#;
        let rich = RichAnswerBatch::from_clef_wire(&batch, body).unwrap();
        let answer = &rich.answers["urgent"];
        assert_eq!(
            answer.answer,
            Answer::Noul {
                yes: true,
                probability: 0.85
            }
        );
        // 1.0 - 0.85 is 0.15000000000000002 in f64; compare approximately.
        assert!((answer.distribution[0] - 0.15).abs() < 1e-9);
        assert!((answer.distribution[1] - 0.85).abs() < 1e-9);
    }

    #[test]
    fn decodes_clef_noul_no() {
        let batch = noul_batch();
        let body = br#"{"model": "clef-flash", "answers": {"urgent": {"type": "noul", "noul": 0.2}}, "usage": {"input_tokens": 10, "output_tokens": 0}}"#;
        let rich = RichAnswerBatch::from_clef_wire(&batch, body).unwrap();
        assert_eq!(
            rich.answers["urgent"].answer,
            Answer::Noul {
                yes: false,
                probability: 0.2
            }
        );
    }

    #[test]
    fn decodes_clef_choice() {
        let batch = choice_batch();
        let body = br#"{"model": "clef-flash", "answers": {"team": {"type": "choice", "choice": "technical", "confidence": 0.92, "probabilities": {"billing": 0.05, "technical": 0.92, "sales": 0.03}}}, "usage": {"input_tokens": 10, "output_tokens": 0}}"#;
        let rich = RichAnswerBatch::from_clef_wire(&batch, body).unwrap();
        let answer = &rich.answers["team"];
        assert_eq!(
            answer.answer,
            Answer::Choice {
                selected: 1,
                probability: 0.92
            }
        );
        assert_eq!(answer.distribution, vec![0.05, 0.92, 0.03]);
    }

    // --- Adversarial: malformed inputs must fail closed ---

    #[test]
    fn rejects_unknown_option_id() {
        let batch = choice_batch();
        let body = br#"{"model": "clef-flash", "answers": {"team": {"type": "choice", "choice": "nonexistent", "confidence": 0.9, "probabilities": {"nonexistent": 0.9}}}, "usage": {}}"#;
        assert!(RichAnswerBatch::from_clef_wire(&batch, body).is_err());
    }

    #[test]
    fn rejects_missing_probability() {
        let batch = choice_batch();
        // "technical" chosen but missing from probabilities dict.
        let body = br#"{"model": "clef-flash", "answers": {"team": {"type": "choice", "choice": "technical", "confidence": 0.9, "probabilities": {"billing": 0.5, "sales": 0.5}}}, "usage": {}}"#;
        assert!(RichAnswerBatch::from_clef_wire(&batch, body).is_err());
    }

    #[test]
    fn rejects_out_of_range_probability() {
        let batch = noul_batch();
        let body = br#"{"model": "clef-flash", "answers": {"urgent": {"type": "noul", "noul": 1.5}}, "usage": {}}"#;
        assert!(RichAnswerBatch::from_clef_wire(&batch, body).is_err());
    }

    #[test]
    fn rejects_wrong_type_tag() {
        let batch = noul_batch();
        let body = br#"{"model": "clef-flash", "answers": {"urgent": {"type": "choice", "choice": "x", "confidence": 0.5, "probabilities": {}}}}, "usage": {}}"#;
        assert!(RichAnswerBatch::from_clef_wire(&batch, body).is_err());
    }

    #[test]
    fn rejects_unasked_question() {
        let batch = noul_batch();
        let body = br#"{"model": "clef-flash", "answers": {"other": {"type": "noul", "noul": 0.5}}, "usage": {}}"#;
        assert!(RichAnswerBatch::from_clef_wire(&batch, body).is_err());
    }

    #[test]
    fn rejects_distribution_not_summing_to_one() {
        let batch = choice_batch();
        // Probabilities are individually in range but sum to 0.75.
        let body = br#"{"model": "clef-flash", "answers": {"team": {"type": "choice", "choice": "technical", "confidence": 0.5, "probabilities": {"billing": 0.1, "technical": 0.5, "sales": 0.15}}}, "usage": {}}"#;
        assert!(RichAnswerBatch::from_clef_wire(&batch, body).is_err());
    }

    // --- RichAnswer::validate: validation + adversarial ---

    fn choice_question() -> Question {
        Question::Choice {
            instructions: "Which team?".to_string(),
            options: vec![
                "billing".to_string(),
                "technical".to_string(),
                "sales".to_string(),
            ],
        }
    }

    #[test]
    fn validate_accepts_a_real_distribution() {
        let rich = RichAnswer {
            answer: Answer::Choice {
                selected: 1,
                probability: 0.92,
            },
            distribution: vec![0.05, 0.92, 0.03],
        };
        assert!(rich.validate(&choice_question()).is_ok());
    }

    #[test]
    fn validate_rejects_wrong_length() {
        let rich = RichAnswer {
            answer: Answer::Choice {
                selected: 1,
                probability: 0.92,
            },
            distribution: vec![0.08, 0.92],
        };
        assert!(rich.validate(&choice_question()).is_err());
    }

    #[test]
    fn validate_rejects_non_finite_and_out_of_range() {
        for distribution in [vec![0.05, f64::NAN, 0.03], vec![-0.1, 0.92, 0.18]] {
            let rich = RichAnswer {
                answer: Answer::Choice {
                    selected: 1,
                    probability: 0.92,
                },
                distribution,
            };
            assert!(rich.validate(&choice_question()).is_err());
        }
    }

    #[test]
    fn validate_rejects_kind_mismatch() {
        let rich = RichAnswer {
            answer: Answer::Noul {
                yes: true,
                probability: 0.9,
            },
            distribution: vec![0.1, 0.9],
        };
        assert!(rich.validate(&choice_question()).is_err());
    }
}
