//! Kev-4B backend: a [`System1Decider`] served by llama.cpp's
//! `/v1/systemone` endpoint running the Kev-4B decision model
//! (adopted under D-1622 as phlow's routing/triage model).
//!
//! # The dialect
//!
//! llama.cpp speaks the Clef form of the system-one protocol, with a
//! request shape of its own (verified against llama.cpp
//! `tools/server/server-decision.cpp` at the pinned build):
//!
//! - the request body is `{"state": s, "questions": {qid: q}}`; there
//!   is no `model` field — the server binds exactly one decision model
//!   at startup, so the endpoint alone identifies what answers;
//! - a Choice question is `{"type": "choice", "instructions": i,
//!   "criteria": {label: description-or-null}}`. A description the
//!   question carries for an option (its `option_descriptions`) is
//!   sent as that option's criterion text; an option without one is
//!   sent as null, so the option key itself is the candidate text.
//!   Option labels must be unique — duplicates would collapse inside
//!   the criteria object, so they are an
//!   [`System1Error::InvalidBatch`] here, before anything is sent;
//! - a Score question is `{"type": "score", "instructions": i,
//!   "criteria": [level, ...]}` with 2..=10 levels (the server's bound,
//!   enforced here as `InvalidBatch`);
//! - a Noul question is `{"type": "noul", "instructions": i}`.
//!
//! The response is the Clef envelope (`{"model", "answers", "usage"}`)
//! and is decoded by [`RichAnswerBatch::from_clef_wire`], with two
//! documented adaptations for this server:
//!
//! - **Score values.** llama.cpp returns a score answer's `score` as
//!   the expected level *index* (0..=levels-1), not the crate's 0..=1
//!   value. [`decide_rich`](KevBackend::decide_rich) divides it by
//!   `levels - 1` before decoding. Probabilities and the server's
//!   confidence pass through untouched.
//! - **Noul scalar probability.** The Clef translation stores P(yes)
//!   in the scalar answer (see `clef.rs`), while [`Answer`] documents
//!   its `probability` as the model's confidence in the answer it
//!   gave. [`decide`](KevBackend::decide) therefore reports
//!   P(yes) for a "yes" and P(no) for a "no", so a confident "no" is
//!   not mistaken for low confidence by confidence-gated consumers.
//!   The rich path is untouched: its distribution stays `[P(no),
//!   P(yes)]` and its scalar keeps the Clef convention, exactly as
//!   the calibration harness expects.
//!
//! Fail-closed throughout: an unreachable server, a non-2xx status,
//! an oversized or malformed body, or an out-of-contract answer is a
//! typed [`System1Error`]; no answer is ever fabricated.

use std::collections::BTreeMap;
use std::fmt;

use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, Url, redirect};
use serde_json::{Map, Value, json};

use crate::backend::{CONNECT_DEADLINE, REQUEST_DEADLINE, System1Decider};
use crate::clef::RichAnswerBatch;
use crate::error::System1Error;
use crate::protocol::{Answer, AnswerBatch, Question, QuestionBatch, RESPONSE_BYTES_MAX};

/// Default Kev endpoint: the estate's `kev-serve` llama.cpp unit on
/// loopback (see `phlow-system1` docs). No remote default exists.
pub const DEFAULT_KEV_ENDPOINT: &str = "http://127.0.0.1:9931";
/// Fewest levels of a Score question the server accepts.
pub const KEV_SCORE_LEVELS_MIN: usize = 2;
/// Most levels of a Score question the server accepts.
pub const KEV_SCORE_LEVELS_MAX: usize = 10;

/// Configuration for [`KevBackend`]: the llama.cpp base endpoint. The
/// served model is fixed by the server at startup, so there is no
/// model field to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KevConfig {
    /// Base endpoint of the llama.cpp server, e.g.
    /// `http://127.0.0.1:9931`.
    pub endpoint: String,
}

impl Default for KevConfig {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_KEV_ENDPOINT.to_owned(),
        }
    }
}

impl KevConfig {
    /// Build a config for an explicit endpoint.
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_owned(),
        }
    }

    fn request_url(&self) -> Result<Url, System1Error> {
        let endpoint = self.endpoint.trim();
        if endpoint.is_empty() {
            return Err(config_error("endpoint must not be empty"));
        }
        let mut url = Url::parse(endpoint).map_err(|_| config_error("endpoint is not a URL"))?;
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(config_error("endpoint scheme must be http or https"));
        }
        if url.host_str().is_none() {
            return Err(config_error("endpoint must name a host"));
        }
        let mut path = url.path().trim_end_matches('/').to_owned();
        path.push_str("/v1/systemone");
        url.set_path(&path);
        Ok(url)
    }
}

fn config_error(reason: &'static str) -> System1Error {
    System1Error::Config { reason }
}

/// A [`System1Decider`] backed by a llama.cpp server running Kev-4B.
pub struct KevBackend {
    client: Client,
    url: Url,
}

impl fmt::Debug for KevBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KevBackend")
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}

impl KevBackend {
    /// Build a backend for `config`; the client is loopback-oriented
    /// (short deadlines, no proxy, no redirects) exactly like the
    /// other HTTP backends in this crate.
    ///
    /// # Errors
    ///
    /// [`System1Error::Config`] if the endpoint is unusable or the
    /// HTTP client cannot be built.
    pub fn new(config: &KevConfig) -> Result<Self, System1Error> {
        let client = Client::builder()
            .timeout(REQUEST_DEADLINE)
            .connect_timeout(CONNECT_DEADLINE)
            .redirect(redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| System1Error::Config {
                reason: "http client could not be built",
            })?;
        Ok(Self {
            client,
            url: config.request_url()?,
        })
    }
}

impl System1Decider for KevBackend {
    async fn decide(&self, batch: &QuestionBatch) -> Result<AnswerBatch, System1Error> {
        Ok(scalar_answers(&self.decide_rich(batch).await?))
    }

    async fn decide_rich(&self, batch: &QuestionBatch) -> Result<RichAnswerBatch, System1Error> {
        let body = to_kev_wire(batch)?;
        let mut response = self
            .client
            .post(self.url.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        if !status.is_success() {
            let detail = format!("server returned HTTP {}", status.as_u16());
            return Err(System1Error::transport(&detail));
        }
        let too_large = || System1Error::protocol("response exceeds RESPONSE_BYTES_MAX");
        if response
            .content_length()
            .is_some_and(|length| length > RESPONSE_BYTES_MAX as u64)
        {
            return Err(too_large());
        }
        let mut received = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            let total = received
                .len()
                .checked_add(chunk.len())
                .ok_or_else(too_large)?;
            if total > RESPONSE_BYTES_MAX {
                return Err(too_large());
            }
            received.extend_from_slice(&chunk);
        }
        decode_kev_response(batch, &received)
    }
}

/// Encode `batch` in the llama.cpp request dialect described in the
/// module docs.
///
/// # Errors
///
/// [`System1Error::InvalidBatch`] if the batch fails validation, a
/// Choice repeats an option label, or a Score question's level count
/// is outside the server's 2..=10 bound.
fn to_kev_wire(batch: &QuestionBatch) -> Result<Vec<u8>, System1Error> {
    batch.validate()?;
    let mut questions = Map::new();
    for (id, question) in &batch.questions {
        let value = match question {
            Question::Choice {
                instructions,
                options,
                option_descriptions,
            } => {
                let mut criteria = Map::new();
                for (index, option) in options.iter().enumerate() {
                    // Validation has already established that the
                    // descriptions are empty or parallel to the
                    // options, so a missing entry here is exactly
                    // "this option has no description": null.
                    let criterion = option_descriptions
                        .get(index)
                        .cloned()
                        .flatten()
                        .map_or(Value::Null, Value::String);
                    if criteria.insert(option.clone(), criterion).is_some() {
                        return Err(System1Error::InvalidBatch {
                            reason: "choice option labels must be unique for the kev dialect",
                        });
                    }
                }
                json!({"type": "choice", "instructions": instructions, "criteria": criteria})
            }
            Question::Score {
                instructions,
                criteria,
            } => {
                if !(KEV_SCORE_LEVELS_MIN..=KEV_SCORE_LEVELS_MAX).contains(&criteria.len()) {
                    return Err(System1Error::InvalidBatch {
                        reason: "kev score questions need 2..=10 levels",
                    });
                }
                json!({"type": "score", "instructions": instructions, "criteria": criteria})
            }
            Question::Noul { instructions } => {
                json!({"type": "noul", "instructions": instructions})
            }
        };
        questions.insert(id.clone(), value);
    }
    serde_json::to_vec(&json!({"state": batch.state, "questions": questions}))
        .map_err(|error| System1Error::protocol(&error.to_string()))
}

/// Decode one llama.cpp response body: normalize score values (see
/// the module docs), then apply the crate's Clef decoder unchanged.
///
/// # Errors
///
/// [`System1Error::Protocol`] for any out-of-contract body, exactly
/// as [`RichAnswerBatch::from_clef_wire`] reports it.
fn decode_kev_response(
    batch: &QuestionBatch,
    body: &[u8],
) -> Result<RichAnswerBatch, System1Error> {
    let body = normalize_score_values(batch, body)?;
    RichAnswerBatch::from_clef_wire(batch, &body)
}

/// Rewrite each score answer's `score` from llama.cpp's expected
/// level index (0..=levels-1) to the crate's 0..=1 value. Bodies
/// without score answers pass through byte-identical; a body that is
/// not the expected envelope shape is left for the Clef decoder to
/// reject, so this step never hides a protocol error.
fn normalize_score_values(batch: &QuestionBatch, body: &[u8]) -> Result<Vec<u8>, System1Error> {
    let levels: BTreeMap<&str, usize> = batch
        .questions
        .iter()
        .filter_map(|(id, question)| match question {
            Question::Score { criteria, .. } if criteria.len() >= 2 => {
                Some((id.as_str(), criteria.len()))
            }
            _ => None,
        })
        .collect();
    if levels.is_empty() {
        return Ok(body.to_vec());
    }
    let mut root: Value =
        serde_json::from_slice(body).map_err(|error| System1Error::protocol(&error.to_string()))?;
    let Some(answers) = root.get_mut("answers").and_then(Value::as_object_mut) else {
        return Ok(body.to_vec());
    };
    for (id, count) in &levels {
        let Some(score) = answers
            .get_mut(*id)
            .and_then(|answer| answer.get_mut("score"))
        else {
            continue;
        };
        if let Some(value) = score.as_f64() {
            // `count` is a protocol-bounded level count (<= 16 by
            // batch validation, <= 10 by the encoder), so the cast
            // and the division are exact and finite.
            *score = json!(value / (*count - 1) as f64);
        }
    }
    serde_json::to_vec(&root).map_err(|error| System1Error::protocol(&error.to_string()))
}

/// Collapse a rich batch to scalar answers, rewriting Noul
/// probabilities to the confidence-in-the-answer-given convention
/// [`Answer`] documents (see the module docs). Choice and Score
/// answers already carry that convention and pass through unchanged.
fn scalar_answers(rich: &RichAnswerBatch) -> AnswerBatch {
    AnswerBatch {
        answers: rich
            .answers
            .iter()
            .map(|(id, rich_answer)| {
                let answer = match rich_answer.answer {
                    Answer::Noul { yes, probability } => Answer::Noul {
                        yes,
                        probability: if yes { probability } else { 1.0 - probability },
                    },
                    other => other,
                };
                (id.clone(), answer)
            })
            .collect(),
    }
}

fn transport_error(error: reqwest::Error) -> System1Error {
    if error.is_timeout() {
        return System1Error::Timeout;
    }
    System1Error::transport(&error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clef::RichAnswer;

    fn batch(state: &str, questions: Vec<(&str, Question)>) -> QuestionBatch {
        QuestionBatch {
            state: state.to_owned(),
            questions: questions
                .into_iter()
                .map(|(id, question)| (id.to_owned(), question))
                .collect(),
        }
    }

    fn choice_batch() -> QuestionBatch {
        batch(
            "Ticket: the export button 500s on large files.",
            vec![(
                "route",
                Question::Choice {
                    instructions: "Which queue should own this ticket?".to_owned(),
                    options: vec!["bug-queue".to_owned(), "feature-queue".to_owned()],
                    option_descriptions: Vec::new(),
                },
            )],
        )
    }

    fn wire_json(batch: &QuestionBatch) -> Value {
        serde_json::from_slice(&to_kev_wire(batch).unwrap()).unwrap()
    }

    #[test]
    fn encodes_choice_criteria_as_null_descriptions() {
        let wire = wire_json(&choice_batch());
        assert_eq!(
            wire["state"],
            "Ticket: the export button 500s on large files."
        );
        let question = &wire["questions"]["route"];
        assert_eq!(question["type"], "choice");
        assert_eq!(
            question["criteria"],
            json!({"bug-queue": null, "feature-queue": null})
        );
        // The llama.cpp dialect has no model field.
        assert!(wire.get("model").is_none());
    }

    #[test]
    fn encodes_described_options_as_criteria_text() {
        let batch = batch(
            "Ticket: the export button 500s on large files.",
            vec![(
                "route",
                Question::Choice {
                    instructions: "Which queue should own this ticket?".to_owned(),
                    options: vec!["bug-queue".to_owned(), "feature-queue".to_owned()],
                    option_descriptions: vec![
                        Some("A defect in shipped behaviour".to_owned()),
                        None,
                    ],
                },
            )],
        );
        let wire = wire_json(&batch);
        assert_eq!(
            wire["questions"]["route"]["criteria"],
            json!({"bug-queue": "A defect in shipped behaviour", "feature-queue": null})
        );
    }

    #[test]
    fn encodes_noul_without_criteria_and_score_levels_as_array() {
        let batch = batch(
            "s",
            vec![
                (
                    "real",
                    Question::Noul {
                        instructions: "Is this a real failure?".to_owned(),
                    },
                ),
                (
                    "risk",
                    Question::Score {
                        instructions: "Rate the risk.".to_owned(),
                        criteria: vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()],
                    },
                ),
            ],
        );
        let wire = wire_json(&batch);
        assert_eq!(
            wire["questions"]["real"],
            json!({"type": "noul", "instructions": "Is this a real failure?"})
        );
        assert_eq!(
            wire["questions"]["risk"]["criteria"],
            json!(["low", "medium", "high"])
        );
    }

    #[test]
    fn duplicate_option_labels_are_invalid_batch() {
        let batch = batch(
            "s",
            vec![(
                "route",
                Question::Choice {
                    instructions: "Pick.".to_owned(),
                    options: vec!["same".to_owned(), "same".to_owned()],
                    option_descriptions: Vec::new(),
                },
            )],
        );
        assert!(matches!(
            to_kev_wire(&batch),
            Err(System1Error::InvalidBatch { .. })
        ));
    }

    #[test]
    fn score_level_counts_outside_the_server_bound_are_invalid_batch() {
        for count in [0_usize, 1, 11] {
            let batch = batch(
                "s",
                vec![(
                    "risk",
                    Question::Score {
                        instructions: "Rate.".to_owned(),
                        criteria: (0..count).map(|i| format!("level-{i}")).collect(),
                    },
                )],
            );
            assert!(
                matches!(to_kev_wire(&batch), Err(System1Error::InvalidBatch { .. })),
                "count {count}"
            );
        }
    }

    #[test]
    fn score_values_are_normalized_before_decode() {
        let batch = batch(
            "s",
            vec![(
                "risk",
                Question::Score {
                    instructions: "Rate the risk.".to_owned(),
                    criteria: vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()],
                },
            )],
        );
        // llama.cpp shape: expected level index 1.5 of 0..=2, i.e. 0.75.
        let body = json!({
            "model": "kev-4b",
            "answers": {"risk": {
                "type": "score", "score": 1.5,
                "legend": {"0": "low", "1": "medium", "2": "high"},
                "probabilities": {"0": 0.25, "1": 0.5, "2": 0.25},
                "confidence": 0.8,
            }},
            "usage": {"input_tokens": 40, "output_tokens": 0},
        });
        let rich = decode_kev_response(&batch, body.to_string().as_bytes()).unwrap();
        assert_eq!(
            rich.answers["risk"].answer,
            Answer::Score {
                value: 0.75,
                confidence: 0.8
            }
        );
        assert_eq!(rich.answers["risk"].distribution, vec![0.25, 0.5, 0.25]);
    }

    #[test]
    fn scalar_noul_probability_is_confidence_in_the_answer_given() {
        let mut rich = RichAnswerBatch {
            answers: BTreeMap::new(),
        };
        rich.answers.insert(
            "no".to_owned(),
            RichAnswer {
                answer: Answer::Noul {
                    yes: false,
                    probability: 0.15,
                },
                distribution: vec![0.85, 0.15],
            },
        );
        rich.answers.insert(
            "yes".to_owned(),
            RichAnswer {
                answer: Answer::Noul {
                    yes: true,
                    probability: 0.9,
                },
                distribution: vec![0.1, 0.9],
            },
        );
        let scalar = scalar_answers(&rich);
        assert_eq!(
            scalar.answers["no"],
            Answer::Noul {
                yes: false,
                probability: 0.85
            }
        );
        assert_eq!(
            scalar.answers["yes"],
            Answer::Noul {
                yes: true,
                probability: 0.9
            }
        );
    }

    #[test]
    fn out_of_contract_bodies_are_protocol_errors() {
        let batch = choice_batch();
        // The bare laya answers object, without the Clef envelope.
        let bare = br#"{"route": {"selected": 0, "probability": 0.9}}"#;
        assert!(matches!(
            decode_kev_response(&batch, bare),
            Err(System1Error::Protocol { .. })
        ));
        // A probability outside 0..=1, as a malfunctioning server might send.
        let bad = json!({
            "model": "kev-4b",
            "answers": {"route": {
                "type": "choice", "choice": "bug-queue",
                "probabilities": {"bug-queue": 1.5, "feature-queue": 0.1},
                "confidence": 0.9,
            }},
            "usage": {"input_tokens": 40, "output_tokens": 0},
        });
        assert!(matches!(
            decode_kev_response(&batch, bad.to_string().as_bytes()),
            Err(System1Error::Protocol { .. })
        ));
        // An option the question never offered.
        let unknown = json!({
            "model": "kev-4b",
            "answers": {"route": {
                "type": "choice", "choice": "other-queue",
                "probabilities": {"other-queue": 0.9},
                "confidence": 0.9,
            }},
            "usage": {"input_tokens": 40, "output_tokens": 0},
        });
        assert!(matches!(
            decode_kev_response(&batch, unknown.to_string().as_bytes()),
            Err(System1Error::Protocol { .. })
        ));
    }
}
