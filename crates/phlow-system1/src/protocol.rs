//! Question and answer shapes, their limits, and the closed wire encoding.
//!
//! Request (sent by [`crate::HttpBackend`]):
//! `{"model": m, "state": s, "questions": {qid: {"type": "Choice"|"Score"|"Noul", ...}}}`.
//! Response: one object keyed by question id; each answer is decoded
//! against the kind of the question it answers:
//! `Choice {"selected": index, "probability": p}`,
//! `Score {"value": v, "confidence": c}`, `Noul {"yes": bool, "probability": p}`.
//! Unknown fields, duplicate ids, missing or unasked ids, out-of-range
//! numbers and oversized bodies are protocol errors, never coerced.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{Deserializer, Error as _, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::System1Error;

/// Maximum questions in one batch (one forward pass).
pub const QUESTIONS_MAX: usize = 16;
/// Maximum bytes of one question id. Ids are `[A-Za-z0-9_-]` only.
pub const QUESTION_ID_BYTES_MAX: usize = 64;
/// Maximum bytes of the shared state string.
pub const STATE_BYTES_MAX: usize = 16 * 1024;
/// Maximum bytes of one question's instructions.
pub const INSTRUCTIONS_BYTES_MAX: usize = 1024;
/// Maximum options of a Choice question or criteria of a Score question.
pub const ITEMS_MAX: usize = 16;
/// Maximum bytes of one option or criterion.
pub const ITEM_BYTES_MAX: usize = 256;
/// Maximum bytes of a server response body.
pub const RESPONSE_BYTES_MAX: usize = 64 * 1024;

/// One question. Serializes to the wire shape, tagged by `type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Question {
    /// Pick exactly one of `options` (at least two).
    Choice {
        instructions: String,
        options: Vec<String>,
    },
    /// Rate on 0.0..=1.0 against `criteria`.
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
    /// Yes or no.
    Noul { instructions: String },
}

/// One shared state plus every question answered in a single forward pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionBatch {
    pub state: String,
    pub questions: BTreeMap<String, Question>,
}

/// One answer. `probability`/`confidence` is the model's confidence in the
/// answer it gave, on 0.0..=1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Answer {
    /// `selected` indexes the question's `options`.
    Choice {
        selected: usize,
        probability: f64,
    },
    /// `value` is on 0.0..=1.0.
    Score {
        value: f64,
        confidence: f64,
    },
    Noul {
        yes: bool,
        probability: f64,
    },
}

impl Answer {
    /// The model's confidence in this answer, whatever its kind.
    pub fn confidence(&self) -> f64 {
        match *self {
            Answer::Choice { probability, .. } | Answer::Noul { probability, .. } => probability,
            Answer::Score { confidence, .. } => confidence,
        }
    }
}

/// Answers keyed by question id.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AnswerBatch {
    pub answers: BTreeMap<String, Answer>,
}

impl QuestionBatch {
    /// Check every named limit and shape rule before anything is sent.
    pub fn validate(&self) -> Result<(), System1Error> {
        let invalid = |reason| Err(System1Error::InvalidBatch { reason });
        if self.state.len() > STATE_BYTES_MAX {
            return invalid("state exceeds STATE_BYTES_MAX");
        }
        if self.questions.is_empty() {
            return invalid("a batch needs at least one question");
        }
        if self.questions.len() > QUESTIONS_MAX {
            return invalid("batch exceeds QUESTIONS_MAX");
        }
        for (id, question) in &self.questions {
            check_id(id)?;
            check_question(question)?;
        }
        Ok(())
    }

    /// Encode the wire request for `model`. Map keys are sorted (BTreeMap),
    /// so equal batches encode to equal bytes.
    pub(crate) fn to_wire(&self, model: &str) -> Result<Vec<u8>, System1Error> {
        self.validate()?;
        let request = serde_json::json!({
            "model": model,
            "state": self.state,
            "questions": self.questions,
        });
        serde_json::to_vec(&request).map_err(|error| System1Error::protocol(&error.to_string()))
    }
}

fn check_id(id: &str) -> Result<(), System1Error> {
    let well_formed = !id.is_empty()
        && id.len() <= QUESTION_ID_BYTES_MAX
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if !well_formed {
        return Err(System1Error::InvalidBatch {
            reason: "question id must be 1..=64 bytes of [A-Za-z0-9_-]",
        });
    }
    Ok(())
}

fn check_question(question: &Question) -> Result<(), System1Error> {
    let (instructions, items, items_min) = match question {
        Question::Choice {
            instructions,
            options,
        } => (instructions, options.as_slice(), 2),
        Question::Score {
            instructions,
            criteria,
        } => (instructions, criteria.as_slice(), 0),
        Question::Noul { instructions } => (instructions, [].as_slice(), 0),
    };
    let invalid = |reason| Err(System1Error::InvalidBatch { reason });
    if instructions.is_empty() || instructions.len() > INSTRUCTIONS_BYTES_MAX {
        return invalid("instructions must be 1..=INSTRUCTIONS_BYTES_MAX bytes");
    }
    if items.len() < items_min || items.len() > ITEMS_MAX {
        return invalid("choice needs 2..=ITEMS_MAX options; criteria at most ITEMS_MAX");
    }
    if items
        .iter()
        .any(|item| item.is_empty() || item.len() > ITEM_BYTES_MAX)
    {
        return invalid("option or criterion must be 1..=ITEM_BYTES_MAX bytes");
    }
    Ok(())
}

// Wire answer shapes, one per question kind. Closed: unknown fields reject.

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChoiceWire {
    selected: u64,
    probability: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreWire {
    value: f64,
    confidence: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoulWire {
    yes: bool,
    probability: f64,
}

impl AnswerBatch {
    /// Decode an untrusted response body for `batch` and validate it.
    ///
    /// Rejects bodies over [`RESPONSE_BYTES_MAX`], duplicate or unasked ids,
    /// answers whose shape does not match their question, and anything
    /// [`AnswerBatch::validate`] rejects.
    pub fn from_wire(batch: &QuestionBatch, body: &[u8]) -> Result<Self, System1Error> {
        if body.len() > RESPONSE_BYTES_MAX {
            return Err(System1Error::protocol(
                "response exceeds RESPONSE_BYTES_MAX",
            ));
        }
        let UniqueAnswers(raw) = serde_json::from_slice(body)
            .map_err(|error| System1Error::protocol(&error.to_string()))?;
        let mut answers = BTreeMap::new();
        for (id, value) in raw {
            let Some(question) = batch.questions.get(&id) else {
                return Err(System1Error::protocol("answer for an unasked question id"));
            };
            answers.insert(id, decode_answer(question, value)?);
        }
        let decoded = AnswerBatch { answers };
        decoded.validate(batch)?;
        Ok(decoded)
    }

    /// Encode these answers in the wire response shape.
    pub fn to_wire(&self) -> Result<Vec<u8>, System1Error> {
        let mut map = serde_json::Map::new();
        for (id, answer) in &self.answers {
            map.insert(id.clone(), encode_answer(answer)?);
        }
        serde_json::to_vec(&Value::Object(map))
            .map_err(|error| System1Error::protocol(&error.to_string()))
    }

    /// Check these answers against `batch`, whatever backend produced them:
    /// exactly one answer per question, of the question's kind, with every
    /// probability, confidence and score finite and on 0.0..=1.0, and every
    /// Choice index inside its options.
    pub fn validate(&self, batch: &QuestionBatch) -> Result<(), System1Error> {
        if self.answers.len() != batch.questions.len() {
            return Err(System1Error::protocol(
                "answer count differs from question count",
            ));
        }
        for (id, question) in &batch.questions {
            let Some(answer) = self.answers.get(id) else {
                return Err(System1Error::protocol(&format!("missing answer for {id}")));
            };
            check_answer(question, answer)
                .map_err(|reason| System1Error::protocol(&format!("answer for {id}: {reason}")))?;
        }
        Ok(())
    }
}

fn unit_interval(number: f64) -> bool {
    number.is_finite() && (0.0..=1.0).contains(&number)
}

fn check_answer(question: &Question, answer: &Answer) -> Result<(), &'static str> {
    if !unit_interval(answer.confidence()) {
        return Err("confidence is not a finite number in 0..=1");
    }
    match (question, answer) {
        (Question::Choice { options, .. }, Answer::Choice { selected, .. }) => {
            if *selected >= options.len() {
                return Err("selected option is out of range");
            }
            Ok(())
        }
        (Question::Score { .. }, Answer::Score { value, .. }) => {
            if !unit_interval(*value) {
                return Err("score is not a finite number in 0..=1");
            }
            Ok(())
        }
        (Question::Noul { .. }, Answer::Noul { .. }) => Ok(()),
        _ => Err("answer kind does not match question kind"),
    }
}

fn decode_answer(question: &Question, value: Value) -> Result<Answer, System1Error> {
    let shape = |error: serde_json::Error| System1Error::protocol(&error.to_string());
    match question {
        Question::Choice { .. } => {
            let wire: ChoiceWire = serde_json::from_value(value).map_err(shape)?;
            let selected = usize::try_from(wire.selected)
                .map_err(|_| System1Error::protocol("selected index overflows usize"))?;
            Ok(Answer::Choice {
                selected,
                probability: wire.probability,
            })
        }
        Question::Score { .. } => {
            let wire: ScoreWire = serde_json::from_value(value).map_err(shape)?;
            Ok(Answer::Score {
                value: wire.value,
                confidence: wire.confidence,
            })
        }
        Question::Noul { .. } => {
            let wire: NoulWire = serde_json::from_value(value).map_err(shape)?;
            Ok(Answer::Noul {
                yes: wire.yes,
                probability: wire.probability,
            })
        }
    }
}

fn encode_answer(answer: &Answer) -> Result<Value, System1Error> {
    let shape = |error: serde_json::Error| System1Error::protocol(&error.to_string());
    match *answer {
        Answer::Choice {
            selected,
            probability,
        } => {
            let selected = u64::try_from(selected)
                .map_err(|_| System1Error::protocol("selected index overflows u64"))?;
            serde_json::to_value(ChoiceWire {
                selected,
                probability,
            })
            .map_err(shape)
        }
        Answer::Score { value, confidence } => {
            serde_json::to_value(ScoreWire { value, confidence }).map_err(shape)
        }
        Answer::Noul { yes, probability } => {
            serde_json::to_value(NoulWire { yes, probability }).map_err(shape)
        }
    }
}

/// A response object whose keys must be unique. serde's `BTreeMap` decode
/// keeps the last duplicate silently, which would let a server send two
/// answers for one id and have a parser-dependent one win.
struct UniqueAnswers(BTreeMap<String, Value>);

impl<'de> Deserialize<'de> for UniqueAnswers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(UniqueAnswersVisitor)
    }
}

struct UniqueAnswersVisitor;

impl<'de> Visitor<'de> for UniqueAnswersVisitor {
    type Value = UniqueAnswers;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object of answers keyed by question id")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut answers = BTreeMap::new();
        while let Some((id, value)) = access.next_entry::<String, Value>()? {
            if answers.len() >= QUESTIONS_MAX {
                return Err(A::Error::custom("more answers than QUESTIONS_MAX"));
            }
            if answers.insert(id, value).is_some() {
                return Err(A::Error::custom("duplicate question id"));
            }
        }
        Ok(UniqueAnswers(answers))
    }
}
