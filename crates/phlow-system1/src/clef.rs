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
        // Validate answer count matches question count.
        rich.to_answer_batch().validate(batch)?;
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
}
