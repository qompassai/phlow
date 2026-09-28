//! Contrastive System-1 decisions: typed questions answered by scaled-cosine
//! scoring over projected state/action vectors.
//!
//! Plain words: instead of asking a language model to pick an action in
//! prose, CLM scores each candidate directly — the scaled cosine between the
//! projected *state* and the projected *candidate*, exactly as in its InfoNCE
//! training objective — and a softmax over a question's candidates is that
//! question's answer distribution (`/tmp/CLM/src/clm/schema.py` module
//! docstring; `/tmp/CLM/src/clm/engine.py`, `Engine::answer`). This module
//! ports the *decision semantics* to Rust: typed question shapes
//! (`noul` / `choice` / `score`), candidate extraction, numerically stable
//! softmax, TypeSafe-style confidence, ranking, and the best-of-N trajectory
//! verifier's selection entry point (see [`crate::best_of_n`]).
//!
//! # Wiring
//!
//! [`SystemOne`] is generic over an [`Embedder`] (text -> embeddings) and a
//! [`ProjectionHead`](phlow_inference::projection::ProjectionHead)
//! (embeddings -> projected vectors). Projected vectors are cached in a
//! [`VectorArena`](phlow_inference::vector_arena::VectorArena) under
//! `{head-namespace}/state` and `{head-namespace}/action`, mirroring
//! `Engine._cached` (`/tmp/CLM/src/clm/engine.py:88-100`). The namespace
//! carries the head's generation, so a hot-reloaded head stops matching its
//! previous rows.
//!
//! # Divergences from CLM
//!
//! - `schema.py`'s `to_text` renders JSON states as prose for the heads; the
//!   Rust port takes pre-rendered state text (`&str`). Callers render their
//!   own context.
//! - There is no multi-model registry here (`Engine.heads`): the head *is*
//!   the model. `SystemOneOutput.model` reports the head's namespace.
//! - There is no HTTP server in this crate, so the TypeSafe routes
//!   (`POST /v1/systemone`, `POST /v1/rank`, `GET /v1/models` in
//!   `/tmp/CLM/src/clm/server.py`) are not implemented; the request/response
//!   *shapes* they validate are.
//! - The embedder has no HTTP implementation yet: embedding is a trait, and
//!   a future transport (Ollama `/api/embeddings`, vLLM pooling) can
//!   implement it. This keeps this crate dependency-free of HTTP clients.
//!
//! # Bounds
//!
//! - [`QUESTIONS_MAX`] questions per call, [`CANDIDATES_MAX`] candidates per
//!   question, [`TEXT_CHARS_MAX`] chars per state/instruction/candidate text.
//! - Temperature must be in `(0, 100]`, mirroring `Engine.answer`
//!   (`/tmp/CLM/src/clm/engine.py:133-134`).
//!
//! # Unsafe policy
//!
//! This module forbids unsafe code (crate-level `#![forbid(unsafe_code)]`).

use std::cmp::Ordering;
use std::fmt;

use phlow_inference::projection::{LOGIT_SCALE_MAX, ProjectionError, ProjectionHead};
use phlow_inference::vector_arena::{CacheError, VectorArena};

/// Maximum questions answered in one [`SystemOne::answer`] call.
pub const QUESTIONS_MAX: usize = 256;

/// Maximum candidates scored for one question.
pub const CANDIDATES_MAX: usize = 4096;

/// Maximum chars of a state, instruction, candidate, or question id.
pub const TEXT_CHARS_MAX: usize = 1 << 16;

/// Maximum question-id chars.
pub const ID_CHARS_MAX: usize = 128;

/// Upper bound for temperature, mirroring `Engine.answer`.
pub const TEMPERATURE_MAX: f32 = 100.0;

/// Bound for error context strings carried in errors.
const ERROR_CONTEXT_CHARS_MAX: usize = 256;

/// The three typed question shapes (`/tmp/CLM/src/clm/schema.py:24`,
/// `QUESTION_TYPES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionKind {
    /// Yes/no statement: answered with P(true).
    Noul,
    /// Pick one of named options.
    Choice,
    /// Expected level over an ordered list of levels.
    Score,
}

/// Per-kind question parameters, mirroring `schema.candidates`
/// (`/tmp/CLM/src/clm/schema.py:45-73`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Criteria {
    /// `(key, description)` pairs in answer order. An empty description
    /// means the option's own key is the candidate text.
    Choice(Vec<(String, String)>),
    /// Ordered levels, at least two.
    Score(Vec<String>),
    /// Optional per-side descriptions; absent descriptions fall back to the
    /// statement templates.
    Noul {
        desc_true: Option<String>,
        desc_false: Option<String>,
    },
}

/// One typed question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// Which shape this question has.
    pub kind: QuestionKind,
    /// The question, appended after the state (`schema.state_text`).
    pub instructions: String,
    /// Per-kind parameters.
    pub criteria: Criteria,
}

/// A question with its state text and candidate texts resolved, ready to
/// score. Mirrors `build_pairs` (`/tmp/CLM/src/clm/schema.py:76-81`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltQuestion {
    /// Question id.
    pub id: String,
    /// State with the question's instructions appended (`state_text`).
    pub state_text: String,
    /// Option keys in answer order.
    pub keys: Vec<String>,
    /// Candidate text per option, in the same order.
    pub candidates: Vec<String>,
    /// Which shape this question has (needed to assemble the answer).
    pub kind: QuestionKind,
    /// Score level texts, in order; empty for non-score questions.
    pub legend: Vec<String>,
}

/// Failures of question construction. All are caller errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionError {
    /// No questions were given.
    EmptyQuestions,
    /// More than [`QUESTIONS_MAX`] questions.
    TooManyQuestions {
        /// Questions offered.
        count: usize,
    },
    /// A question id was empty or over [`ID_CHARS_MAX`] chars.
    BadId,
    /// Two questions shared an id.
    DuplicateId,
    /// A choice question had no options.
    EmptyChoiceCriteria,
    /// A score question had fewer than two levels.
    ScoreTooFewLevels {
        /// Levels offered.
        count: usize,
    },
    /// A question had more than [`CANDIDATES_MAX`] candidates.
    TooManyCandidates {
        /// Candidates built.
        count: usize,
    },
    /// A state, instruction, description, or candidate text exceeded
    /// [`TEXT_CHARS_MAX`] chars.
    TextTooLong,
    /// No candidates to rank.
    NoCandidates,
    /// A rank candidate was not a non-empty string (mirrors the server's
    /// 422 "answers must be non-empty strings",
    /// `/tmp/CLM/src/clm/server.py:130-131`).
    BadCandidate,
}

impl fmt::Display for QuestionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuestionError::EmptyQuestions => write!(f, "questions must not be empty"),
            QuestionError::TooManyQuestions { count } => {
                write!(f, "{count} questions exceeds QUESTIONS_MAX={QUESTIONS_MAX}")
            }
            QuestionError::BadId => write!(f, "question id must be 1..=ID_CHARS_MAX chars"),
            QuestionError::DuplicateId => write!(f, "duplicate question id"),
            QuestionError::EmptyChoiceCriteria => {
                write!(f, "choice question needs a non-empty 'criteria' object")
            }
            QuestionError::ScoreTooFewLevels { count } => {
                write!(f, "score question needs >= 2 levels, got {count}")
            }
            QuestionError::TooManyCandidates { count } => {
                write!(
                    f,
                    "{count} candidates exceeds CANDIDATES_MAX={CANDIDATES_MAX}"
                )
            }
            QuestionError::TextTooLong => {
                write!(f, "text exceeds TEXT_CHARS_MAX={TEXT_CHARS_MAX} chars")
            }
            QuestionError::NoCandidates => write!(f, "no candidates to rank"),
            QuestionError::BadCandidate => write!(f, "candidates must be non-empty strings"),
        }
    }
}

impl std::error::Error for QuestionError {}

/// Failures of scoring. All are caller errors.
#[derive(Debug, Clone, PartialEq)]
pub enum ScoringError {
    /// Temperature was not in `(0, 100]`.
    BadTemperature {
        /// The offending value.
        value: f32,
    },
    /// Scale was not finite and positive.
    BadScale {
        /// The offending value.
        value: f32,
    },
    /// A candidate vector did not match the state vector's width.
    DimMismatch {
        /// State width.
        expected: usize,
        /// Candidate width.
        got: usize,
    },
    /// A vector was empty.
    EmptyVector,
    /// A vector component or probability was non-finite.
    NonFinite,
    /// Candidate count did not match the question's option count.
    CountMismatch {
        /// Options in the question.
        options: usize,
        /// Vectors offered.
        vectors: usize,
    },
    /// Probability/candidate slices had different lengths.
    RankLengthMismatch,
}

impl fmt::Display for ScoringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScoringError::BadTemperature { value } => {
                write!(f, "temperature must be in (0, 100], got {value}")
            }
            ScoringError::BadScale { value } => {
                write!(f, "scale must be finite and positive, got {value}")
            }
            ScoringError::DimMismatch { expected, got } => {
                write!(
                    f,
                    "candidate width {got} does not match state width {expected}"
                )
            }
            ScoringError::EmptyVector => write!(f, "scoring vectors must not be empty"),
            ScoringError::NonFinite => write!(f, "scoring vectors must be finite"),
            ScoringError::CountMismatch { options, vectors } => {
                write!(f, "{vectors} vectors for {options} options")
            }
            ScoringError::RankLengthMismatch => {
                write!(f, "candidates and probabilities must have equal length")
            }
        }
    }
}

impl std::error::Error for ScoringError {}

/// Bound a message for inclusion in error context.
fn bound_context(message: &str) -> String {
    message.chars().take(ERROR_CONTEXT_CHARS_MAX).collect()
}

/// Failures of embedding. Transport messages are bounded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedError {
    /// The encoder transport failed.
    Transport(String),
    /// The encoder answered with unusable rows.
    BadResponse(String),
}

impl EmbedError {
    /// Build a transport failure with bounded context.
    pub fn transport(message: &str) -> Self {
        Self::Transport(bound_context(message))
    }

    /// Build a bad-response failure with bounded context.
    pub fn bad_response(message: &str) -> Self {
        Self::BadResponse(bound_context(message))
    }
}

impl fmt::Display for EmbedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmbedError::Transport(message) => write!(f, "embedder transport failed: {message}"),
            EmbedError::BadResponse(message) => write!(f, "embedder bad response: {message}"),
        }
    }
}

impl std::error::Error for EmbedError {}

/// Encoder embeddings for a batch of texts.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedded {
    /// One embedding row per input text, in order.
    pub vectors: Vec<Vec<f32>>,
    /// Encoder tokens spent (cache misses only, like CLM's usage
    /// accounting).
    pub tokens: u64,
}

/// Text -> L2-normalised encoder embeddings. No HTTP implementation ships
/// with this crate (see module docs); the trait keeps the scorer
/// transport-agnostic.
pub trait Embedder {
    /// Embed `texts` in order.
    fn embed(&self, texts: &[String]) -> Result<Embedded, EmbedError>;
}

/// Failures of [`SystemOne`]. Wraps the stage that failed.
#[derive(Debug, Clone, PartialEq)]
pub enum SystemOneError {
    /// Question construction failed.
    Question(QuestionError),
    /// Scoring failed.
    Scoring(ScoringError),
    /// Embedding failed.
    Embed(EmbedError),
    /// Projection failed.
    Projection(ProjectionError),
    /// Vector-cache lookup failed.
    Cache(CacheError),
}

impl fmt::Display for SystemOneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SystemOneError::Question(e) => write!(f, "invalid question: {e}"),
            SystemOneError::Scoring(e) => write!(f, "scoring failed: {e}"),
            SystemOneError::Embed(e) => write!(f, "{e}"),
            SystemOneError::Projection(e) => write!(f, "projection failed: {e}"),
            SystemOneError::Cache(e) => write!(f, "vector cache failed: {e}"),
        }
    }
}

impl std::error::Error for SystemOneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SystemOneError::Question(e) => Some(e),
            SystemOneError::Scoring(e) => Some(e),
            SystemOneError::Embed(e) => Some(e),
            SystemOneError::Projection(e) => Some(e),
            SystemOneError::Cache(e) => Some(e),
        }
    }
}

impl From<QuestionError> for SystemOneError {
    fn from(e: QuestionError) -> Self {
        SystemOneError::Question(e)
    }
}

impl From<ScoringError> for SystemOneError {
    fn from(e: ScoringError) -> Self {
        SystemOneError::Scoring(e)
    }
}

/// Check a text against the char bound.
fn check_text(text: &str) -> Result<(), QuestionError> {
    if text.chars().count() > TEXT_CHARS_MAX {
        return Err(QuestionError::TextTooLong);
    }
    Ok(())
}

/// `context + question` layout the heads are trained on: context first,
/// question last (`/tmp/CLM/src/clm/schema.py:40-43`, `state_text`).
fn state_text(state: &str, instructions: &str) -> String {
    let (state, instructions) = (state.trim(), instructions.trim());
    match (state.is_empty(), instructions.is_empty()) {
        (true, true) => String::new(),
        (true, false) => instructions.to_string(),
        (false, true) => state.to_string(),
        (false, false) => format!("{state}\n\n{instructions}"),
    }
}

/// Per-option material for one question: answer keys in answer order,
/// candidate (action) texts in the same order, and legend labels
/// (score levels, else empty), mirroring `schema.candidates`
/// (`/tmp/CLM/src/clm/schema.py:45-73`).
type OptionMaterial = (Vec<String>, Vec<String>, Vec<String>);

/// Option keys in answer order and candidate text per option, mirroring
/// `schema.candidates` (`/tmp/CLM/src/clm/schema.py:45-73`).
fn candidates(question: &Question) -> Result<OptionMaterial, QuestionError> {
    let instructions = question.instructions.trim();
    match (&question.kind, &question.criteria) {
        (QuestionKind::Choice, Criteria::Choice(options)) => {
            if options.is_empty() {
                return Err(QuestionError::EmptyChoiceCriteria);
            }
            let mut keys = Vec::with_capacity(options.len());
            let mut texts = Vec::with_capacity(options.len());
            for (key, description) in options {
                // The action head embeds the option's own text: its
                // description when one is given, else the key (schema.py).
                let text = if description.is_empty() {
                    key.clone()
                } else {
                    description.clone()
                };
                check_text(&text)?;
                keys.push(key.clone());
                texts.push(text);
            }
            Ok((keys, texts, Vec::new()))
        }
        (QuestionKind::Score, Criteria::Score(levels)) => {
            if levels.len() < 2 {
                return Err(QuestionError::ScoreTooFewLevels {
                    count: levels.len(),
                });
            }
            let mut texts = Vec::with_capacity(levels.len());
            for level in levels {
                check_text(level)?;
                texts.push(level.clone());
            }
            let keys: Vec<String> = (0..levels.len()).map(|i| i.to_string()).collect();
            Ok((keys, texts, levels.clone()))
        }
        (
            QuestionKind::Noul,
            Criteria::Noul {
                desc_true,
                desc_false,
            },
        ) => {
            let mut texts = Vec::with_capacity(2);
            for (key, desc) in [("false", desc_false), ("true", desc_true)] {
                let body = match desc {
                    Some(custom) if !custom.is_empty() => custom.clone(),
                    _ if instructions.is_empty() => key.to_string(),
                    _ if key == "true" => format!("Yes. This is true: {instructions}"),
                    _ => format!("No. This is false: {instructions}"),
                };
                let text = format!("{key}: {body}");
                check_text(&text)?;
                texts.push(text);
            }
            Ok((
                vec!["false".to_string(), "true".to_string()],
                texts,
                Vec::new(),
            ))
        }
        _ => Err(QuestionError::EmptyChoiceCriteria),
    }
}

/// Resolve every question to its state text and candidate texts. Mirrors
/// `build_pairs` (`/tmp/CLM/src/clm/schema.py:76-81`).
pub fn build_pairs(
    state: &str,
    questions: &[(&str, Question)],
) -> Result<Vec<BuiltQuestion>, QuestionError> {
    if questions.is_empty() {
        return Err(QuestionError::EmptyQuestions);
    }
    if questions.len() > QUESTIONS_MAX {
        return Err(QuestionError::TooManyQuestions {
            count: questions.len(),
        });
    }
    check_text(state)?;
    let mut seen: Vec<&str> = Vec::with_capacity(questions.len());
    let mut built = Vec::with_capacity(questions.len());
    for (id, question) in questions {
        if id.is_empty() || id.chars().count() > ID_CHARS_MAX {
            return Err(QuestionError::BadId);
        }
        if seen.contains(id) {
            return Err(QuestionError::DuplicateId);
        }
        seen.push(id);
        check_text(&question.instructions)?;
        let (keys, candidates, legend) = candidates(question)?;
        if candidates.len() > CANDIDATES_MAX {
            return Err(QuestionError::TooManyCandidates {
                count: candidates.len(),
            });
        }
        built.push(BuiltQuestion {
            id: (*id).to_string(),
            state_text: state_text(state, &question.instructions),
            keys,
            candidates,
            kind: question.kind,
            legend,
        });
    }
    Ok(built)
}

/// Numerically stable softmax: subtract the max before exponentiating.
/// Mirrors `schema.softmax` (`/tmp/CLM/src/clm/schema.py:84-88`).
pub fn softmax(logits: &[f32]) -> Vec<f32> {
    if logits.is_empty() {
        return Vec::new();
    }
    let max = logits.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let exps: Vec<f32> = logits.iter().map(|&v| (v - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    // `sum` is positive: every exp is positive and the slice is non-empty.
    exps.iter().map(|&e| e / sum).collect()
}

/// TypeSafe-style confidence: top probability minus the mean of the rest,
/// clamped to [0, 1]; 1.0 when fewer than two options. Mirrors
/// `schema.confidence` (`/tmp/CLM/src/clm/schema.py:91-97`).
pub fn confidence(probs: &[f32]) -> f32 {
    if probs.len() < 2 {
        return 1.0;
    }
    let top = probs.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let rest_sum: f32 = probs.iter().sum::<f32>() - top;
    let rest_mean = rest_sum / (probs.len() - 1) as f32;
    (top - rest_mean).clamp(0.0, 1.0)
}

/// Cosine similarity of two L2-normalised vectors.
fn cosine(state: &[f32], candidate: &[f32]) -> f32 {
    state
        .iter()
        .zip(candidate.iter())
        .map(|(&a, &b)| f64::from(a) * f64::from(b))
        .sum::<f64>() as f32
}

/// An answered question, mirroring the Answer objects `answer_from_probs`
/// assembles (`/tmp/CLM/src/clm/schema.py:100-114`).
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// P(true) for the statement.
    Noul {
        /// Probability the statement is true.
        prob_true: f32,
    },
    /// Winning option key, confidence, and the full distribution.
    Choice {
        /// Winning option key.
        choice: String,
        /// TypeSafe-style confidence.
        confidence: f32,
        /// `(key, probability)` in answer order.
        probabilities: Vec<(String, f32)>,
    },
    /// Expected level, confidence, legend, and the full distribution.
    Score {
        /// Expected level: `sum(i * p_i)`.
        score: f32,
        /// TypeSafe-style confidence.
        confidence: f32,
        /// `(level index, level text)` in order.
        legend: Vec<(String, String)>,
        /// `(level index, probability)` in order.
        probabilities: Vec<(String, f32)>,
    },
}

/// Discrete label of an answer, for scoring. Mirrors `label_of`
/// (`/tmp/CLM/src/clm/schema.py:117-125`).
pub fn label_of(answer: &Answer) -> String {
    match answer {
        Answer::Choice { choice, .. } => choice.clone(),
        Answer::Noul { prob_true } => {
            if *prob_true >= 0.5 {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Answer::Score { probabilities, .. } => probabilities
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
            .map(|(key, _)| key.clone())
            .unwrap_or_default(),
    }
}

/// Full option distribution of an answer. Mirrors `probabilities_of`
/// (`/tmp/CLM/src/clm/schema.py:128-131`).
pub fn probabilities_of(answer: &Answer) -> Vec<(String, f32)> {
    match answer {
        Answer::Noul { prob_true } => {
            vec![
                ("false".to_string(), 1.0 - prob_true),
                ("true".to_string(), *prob_true),
            ]
        }
        Answer::Choice { probabilities, .. } | Answer::Score { probabilities, .. } => {
            probabilities.clone()
        }
    }
}

/// Score one question: `softmax(scale * cosine / temperature)` over its
/// candidates, assembled into the question's answer shape. Mirrors the
/// per-question loop in `Engine.answer`
/// (`/tmp/CLM/src/clm/engine.py:149-153`).
pub fn score_question(
    built: &BuiltQuestion,
    state_vec: &[f32],
    candidate_vecs: &[Vec<f32>],
    scale: f32,
    temperature: f32,
) -> Result<Answer, ScoringError> {
    if !temperature.is_finite() || temperature <= 0.0 || temperature > TEMPERATURE_MAX {
        return Err(ScoringError::BadTemperature { value: temperature });
    }
    if !scale.is_finite() || scale <= 0.0 || scale > LOGIT_SCALE_MAX {
        return Err(ScoringError::BadScale { value: scale });
    }
    if state_vec.is_empty() {
        return Err(ScoringError::EmptyVector);
    }
    if candidate_vecs.len() != built.keys.len() {
        return Err(ScoringError::CountMismatch {
            options: built.keys.len(),
            vectors: candidate_vecs.len(),
        });
    }
    if !state_vec.iter().all(|x| x.is_finite()) {
        return Err(ScoringError::NonFinite);
    }
    let dim = state_vec.len();
    for candidate in candidate_vecs {
        if candidate.len() != dim {
            return Err(ScoringError::DimMismatch {
                expected: dim,
                got: candidate.len(),
            });
        }
        if !candidate.iter().all(|x| x.is_finite()) {
            return Err(ScoringError::NonFinite);
        }
    }
    let logits: Vec<f32> = candidate_vecs
        .iter()
        .map(|v| scale * cosine(state_vec, v) / temperature)
        .collect();
    let probs = softmax(&logits);
    let dist: Vec<(String, f32)> = built
        .keys
        .iter()
        .cloned()
        .zip(probs.iter().copied())
        .collect();
    match built.kind {
        QuestionKind::Noul => {
            // Keys are ["false", "true"] in that order (see `candidates`).
            let prob_true = dist
                .iter()
                .find(|(key, _)| key == "true")
                .map(|(_, p)| *p)
                .unwrap_or(0.0);
            Ok(Answer::Noul { prob_true })
        }
        QuestionKind::Choice => {
            // First maximum wins, like Python's max(range, key=...).
            let mut best = 0;
            for (i, &p) in probs.iter().enumerate() {
                if p > probs[best] {
                    best = i;
                }
            }
            Ok(Answer::Choice {
                choice: built.keys[best].clone(),
                confidence: confidence(&probs),
                probabilities: dist,
            })
        }
        QuestionKind::Score => {
            let score: f32 = probs.iter().enumerate().map(|(i, &p)| i as f32 * p).sum();
            let legend: Vec<(String, String)> = built
                .legend
                .iter()
                .enumerate()
                .map(|(i, level)| (i.to_string(), level.clone()))
                .collect();
            Ok(Answer::Score {
                score,
                confidence: confidence(&probs),
                legend,
                probabilities: dist,
            })
        }
    }
}

/// One ranked candidate, best first. Ranks are 1-based, mirroring
/// `Engine.rank` (`/tmp/CLM/src/clm/engine.py:169-172`).
#[derive(Debug, Clone, PartialEq)]
pub struct RankEntry {
    /// 1-based rank.
    pub rank: usize,
    /// Candidate text.
    pub candidate: String,
    /// Its probability.
    pub prob: f32,
}

/// Order `(candidate, probability)` pairs best-first. Sort is stable, so
/// exact ties keep input order — the same guarantee as Python's `sorted`
/// in `Engine.rank`.
pub fn rank_pairs(pairs: &[(String, f32)]) -> Result<Vec<RankEntry>, ScoringError> {
    if pairs.iter().any(|(_, p)| !p.is_finite()) {
        return Err(ScoringError::NonFinite);
    }
    let mut order: Vec<usize> = (0..pairs.len()).collect();
    // Stable: ties keep input order.
    order.sort_by(|&a, &b| {
        pairs[b]
            .1
            .partial_cmp(&pairs[a].1)
            .unwrap_or(Ordering::Equal)
    });
    Ok(order
        .iter()
        .enumerate()
        .map(|(position, &i)| RankEntry {
            rank: position + 1,
            candidate: pairs[i].0.clone(),
            prob: pairs[i].1,
        })
        .collect())
}

/// Usage accounting, mirroring `Engine.answer`'s envelope
/// (`/tmp/CLM/src/clm/engine.py:154-155`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// Questions answered (the billing unit).
    pub billing_units: usize,
    /// Encoder tokens spent (cache misses only).
    pub input_tokens: u64,
    /// Always zero: scoring emits no tokens.
    pub output_tokens: u64,
}

/// The answered-questions envelope: `{"model", "answers", "usage"}`.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemOneOutput {
    /// The head namespace that answered.
    pub model: String,
    /// `(question id, answer)` in request order.
    pub answers: Vec<(String, Answer)>,
    /// Token/billing accounting.
    pub usage: Usage,
}

/// Which side of the contrastive pair is being projected. CLM uses
/// independent heads for states and actions (`/tmp/CLM/src/clm/heads.py`),
/// so the two namespaces must route to different `ProjectionHead` methods.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProjectionSide {
    State,
    Action,
}

/// Contrastive System-1 answering over an embedder and a projection head.
///
/// `E` turns texts into encoder embeddings; `H` projects them into the
/// head's space. Projected state/action vectors are cached in `arena`
/// (when present) under `{head-namespace}/state` and `{head-namespace}/action`,
/// mirroring `Engine._cached`.
pub struct SystemOne<E: Embedder, H: ProjectionHead> {
    embedder: E,
    head: H,
    arena: Option<VectorArena>,
}

impl<E: Embedder, H: ProjectionHead> SystemOne<E, H> {
    /// Build the answerer. `arena` may be `None` to disable vector caching
    /// (like `CLM_ACTION_CACHE=0`).
    pub fn new(embedder: E, head: H, arena: Option<VectorArena>) -> Self {
        Self {
            embedder,
            head,
            arena,
        }
    }

    /// Embed and project `texts` on `side`, charging encoder tokens.
    /// Uncached path: embed/projection failures keep their typed errors.
    fn project_missing(
        &self,
        texts: &[String],
        tokens: &mut u64,
        embed_dim: usize,
        side: ProjectionSide,
    ) -> Result<Vec<Vec<f32>>, SystemOneError> {
        let embedded = self.embedder.embed(texts).map_err(SystemOneError::Embed)?;
        *tokens += embedded.tokens;
        if embedded.vectors.len() != texts.len() {
            return Err(SystemOneError::Embed(EmbedError::bad_response(
                "embedder returned a row-count mismatch",
            )));
        }
        for (index, row) in embedded.vectors.iter().enumerate() {
            if row.len() != embed_dim || row.iter().any(|x| !x.is_finite()) {
                let message = format!("embedder row {index} has bad width or non-finite value");
                return Err(SystemOneError::Embed(EmbedError::bad_response(&message)));
            }
        }
        let projected = match side {
            ProjectionSide::State => self.head.project_states(&embedded.vectors),
            ProjectionSide::Action => self.head.project_actions(&embedded.vectors),
        };
        projected.map_err(SystemOneError::Projection)
    }

    /// Projected vectors for `texts` under `namespace`, from the arena where
    /// possible. `tokens` collects encoder tokens spent on misses. The arena
    /// path wraps compute failures in [`CacheError`]; without an arena the
    /// typed [`SystemOneError::Embed`]/[`SystemOneError::Projection`]
    /// variants are preserved.
    fn cached_vectors(
        &self,
        namespace: &str,
        texts: &[String],
        tokens: &mut u64,
        embed_dim: usize,
        project_dim: usize,
        side: ProjectionSide,
    ) -> Result<Vec<Vec<f32>>, SystemOneError> {
        match &self.arena {
            Some(arena) => {
                let mut project = |missing: &[String]| -> Result<Vec<Vec<f32>>, CacheError> {
                    self.project_missing(missing, tokens, embed_dim, side)
                        .map_err(|e| VectorArena::compute_failed(&e.to_string()))
                };
                arena
                    .get(namespace, project_dim, texts, &mut project)
                    .map_err(SystemOneError::Cache)
            }
            None => self.project_missing(texts, tokens, embed_dim, side),
        }
    }

    /// Answer typed questions about `state`. Mirrors `Engine.answer`
    /// (`/tmp/CLM/src/clm/engine.py:127-155`): validates, builds pairs,
    /// embeds + projects states and candidates (cached), then scores each
    /// question as `softmax(scale * cosine / temperature)`.
    pub fn answer(
        &self,
        state: &str,
        questions: &[(&str, Question)],
        temperature: f32,
    ) -> Result<SystemOneOutput, SystemOneError> {
        if !temperature.is_finite() || temperature <= 0.0 || temperature > TEMPERATURE_MAX {
            return Err(SystemOneError::Scoring(ScoringError::BadTemperature {
                value: temperature,
            }));
        }
        let built = build_pairs(state, questions)?;
        let namespace = self.head.namespace();
        let scale = self.head.logit_scale();
        let embed_dim = self.head.embed_dim();
        let project_dim = self.head.projection_dim();

        let state_texts: Vec<String> = built.iter().map(|b| b.state_text.clone()).collect();
        let mut tokens = 0u64;
        let state_vecs = self.cached_vectors(
            &format!("{namespace}/state"),
            &state_texts,
            &mut tokens,
            embed_dim,
            project_dim,
            ProjectionSide::State,
        )?;
        let candidate_texts: Vec<String> = built
            .iter()
            .flat_map(|b| b.candidates.iter().cloned())
            .collect();
        let candidate_vecs = self.cached_vectors(
            &format!("{namespace}/action"),
            &candidate_texts,
            &mut tokens,
            embed_dim,
            project_dim,
            ProjectionSide::Action,
        )?;

        let mut answers = Vec::with_capacity(built.len());
        let mut offset = 0;
        for (index, question) in built.iter().enumerate() {
            let count = question.candidates.len();
            let vectors = &candidate_vecs[offset..offset + count];
            offset += count;
            let answer = score_question(question, &state_vecs[index], vectors, scale, temperature)?;
            answers.push((question.id.clone(), answer));
        }
        Ok(SystemOneOutput {
            model: namespace,
            answers,
            usage: Usage {
                billing_units: built.len(),
                input_tokens: tokens,
                output_tokens: 0,
            },
        })
    }

    /// Rank free-form candidate strings against a state, best first.
    /// Mirrors `Engine.rank` (`/tmp/CLM/src/clm/engine.py:157-172`): the
    /// state head sees `context + question`, the action head sees each
    /// candidate verbatim, via a synthetic choice question.
    pub fn rank(
        &self,
        context: &str,
        candidates: &[String],
        instructions: &str,
        temperature: f32,
    ) -> Result<Vec<RankEntry>, SystemOneError> {
        if candidates.is_empty() {
            return Err(SystemOneError::Question(QuestionError::NoCandidates));
        }
        for candidate in candidates {
            if candidate.is_empty() || candidate.chars().count() > TEXT_CHARS_MAX {
                return Err(SystemOneError::Question(QuestionError::BadCandidate));
            }
        }
        let options: Vec<(String, String)> = candidates
            .iter()
            .enumerate()
            .map(|(i, text)| (i.to_string(), text.clone()))
            .collect();
        let question = Question {
            kind: QuestionKind::Choice,
            instructions: instructions.to_string(),
            criteria: Criteria::Choice(options),
        };
        let output = self.answer(context, &[("rank", question)], temperature)?;
        // Internal invariant: one question in, one answer out, and it is the
        // synthetic choice question.
        assert_eq!(
            output.answers.len(),
            1,
            "answer returns one answer per question"
        );
        let (_, answer) = &output.answers[0];
        match answer {
            Answer::Choice { probabilities, .. } => {
                let pairs: Vec<(String, f32)> = probabilities
                    .iter()
                    .map(|(key, prob)| {
                        let index: usize = key.parse().unwrap_or(usize::MAX);
                        let candidate = candidates.get(index).cloned().unwrap_or_default();
                        (candidate, *prob)
                    })
                    .collect();
                Ok(rank_pairs(&pairs)?)
            }
            _ => unreachable!("synthetic rank question always answers Choice"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use phlow_inference::projection::HashProjection;
    use phlow_inference::vector_arena::Budget;

    /// Embedder test double: deterministic vectors derived from text length,
    /// like the arena tests' compute closure.
    struct FakeEmbedder {
        dim: usize,
    }

    impl Embedder for FakeEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Embedded, EmbedError> {
            let mut vectors = Vec::with_capacity(texts.len());
            for text in texts {
                let seed = text.len() as f32;
                vectors.push((0..self.dim).map(|i| seed + i as f32 * 0.001).collect());
            }
            Ok(Embedded {
                vectors,
                tokens: texts.len() as u64,
            })
        }
    }

    fn system_one() -> SystemOne<FakeEmbedder, HashProjection> {
        let head = HashProjection::new("test-head", 8, 16, 28.0).unwrap();
        let arena = VectorArena::new(Budget::Bytes(1 << 20)).unwrap();
        arena.reserve(16, 1.0).unwrap();
        SystemOne::new(FakeEmbedder { dim: 8 }, head, Some(arena))
    }

    fn choice_question() -> Question {
        Question {
            kind: QuestionKind::Choice,
            instructions: "Pick one".to_string(),
            criteria: Criteria::Choice(vec![
                ("a".to_string(), "first option".to_string()),
                ("b".to_string(), String::new()),
            ]),
        }
    }

    // ---- validation: question shapes ----

    #[test]
    fn noul_defaults_use_statement_templates() {
        let question = Question {
            kind: QuestionKind::Noul,
            instructions: "Is it fine?".to_string(),
            criteria: Criteria::Noul {
                desc_true: None,
                desc_false: None,
            },
        };
        let built = build_pairs("state", &[("q", question)]).unwrap();
        assert_eq!(built[0].keys, vec!["false".to_string(), "true".to_string()]);
        assert_eq!(
            built[0].candidates,
            vec![
                "false: No. This is false: Is it fine?".to_string(),
                "true: Yes. This is true: Is it fine?".to_string(),
            ]
        );
    }

    #[test]
    fn choice_uses_description_or_key() {
        let built = build_pairs("s", &[("q", choice_question())]).unwrap();
        assert_eq!(
            built[0].candidates,
            vec!["first option".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn score_keys_and_legend() {
        let question = Question {
            kind: QuestionKind::Score,
            instructions: String::new(),
            criteria: Criteria::Score(vec!["low".to_string(), "high".to_string()]),
        };
        let built = build_pairs("s", &[("q", question)]).unwrap();
        assert_eq!(built[0].keys, vec!["0".to_string(), "1".to_string()]);
        assert_eq!(built[0].legend, vec!["low".to_string(), "high".to_string()]);
    }

    #[test]
    fn state_text_layout_is_context_first_question_last() {
        let built = build_pairs("  ctx  ", &[("q", choice_question())]).unwrap();
        assert_eq!(built[0].state_text, "ctx\n\nPick one");
    }

    #[test]
    fn softmax_sums_to_one_and_is_stable() {
        let probs = softmax(&[1000.0, 1001.0, 999.0]);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6, "sum was {sum}");
        assert!(probs[1] > probs[0] && probs[0] > probs[2]);
    }

    #[test]
    fn confidence_is_top_minus_mean_rest() {
        assert!((confidence(&[0.7, 0.2, 0.1]) - 0.55).abs() < 1e-6);
        assert_eq!(confidence(&[0.5]), 1.0);
    }

    #[test]
    fn rank_orders_best_first_with_stable_ties() {
        let pairs = vec![
            ("a".to_string(), 0.2),
            ("b".to_string(), 0.5),
            ("c".to_string(), 0.5),
        ];
        let ranked = rank_pairs(&pairs).unwrap();
        assert_eq!(ranked[0].rank, 1);
        assert_eq!(ranked[0].candidate, "b");
        // Exact tie: input order kept, like Python's stable sorted.
        assert_eq!(ranked[1].candidate, "c");
        assert_eq!(ranked[2].candidate, "a");
    }

    #[test]
    fn answer_end_to_end_envelope() {
        let one = system_one();
        let output = one
            .answer("the build is green", &[("q", choice_question())], 1.0)
            .unwrap();
        assert_eq!(output.model, "test-head@0");
        assert_eq!(output.answers.len(), 1);
        assert_eq!(output.usage.billing_units, 1);
        assert_eq!(output.usage.output_tokens, 0);
        // 1 state + 2 candidates embedded once each.
        assert_eq!(output.usage.input_tokens, 3);
        match &output.answers[0].1 {
            Answer::Choice {
                choice,
                probabilities,
                ..
            } => {
                assert!(choice == "a" || choice == "b");
                let sum: f32 = probabilities.iter().map(|(_, p)| p).sum();
                assert!((sum - 1.0).abs() < 1e-5);
            }
            other => panic!("expected Choice, got {other:?}"),
        }
    }

    // ---- adversarial: invalid questions and scoring inputs ----

    #[test]
    fn empty_questions_rejected() {
        assert_eq!(build_pairs("s", &[]), Err(QuestionError::EmptyQuestions));
    }

    #[test]
    fn temperature_bounds_rejected() {
        let one = system_one();
        for bad in [0.0, -1.0, 101.0, f32::NAN, f32::INFINITY] {
            let err = one
                .answer("s", &[("q", choice_question())], bad)
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    SystemOneError::Scoring(ScoringError::BadTemperature { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn choice_empty_criteria_rejected() {
        let question = Question {
            kind: QuestionKind::Choice,
            instructions: String::new(),
            criteria: Criteria::Choice(vec![]),
        };
        assert_eq!(
            build_pairs("s", &[("q", question)]),
            Err(QuestionError::EmptyChoiceCriteria)
        );
    }

    #[test]
    fn score_single_level_rejected() {
        let question = Question {
            kind: QuestionKind::Score,
            instructions: String::new(),
            criteria: Criteria::Score(vec!["only".to_string()]),
        };
        assert_eq!(
            build_pairs("s", &[("q", question)]),
            Err(QuestionError::ScoreTooFewLevels { count: 1 })
        );
    }

    #[test]
    fn mismatched_vector_dims_rejected() {
        let built = build_pairs("s", &[("q", choice_question())]).unwrap();
        let state = vec![1.0, 0.0];
        let candidates = vec![vec![1.0, 0.0], vec![1.0]];
        let err = score_question(&built[0], &state, &candidates, 28.0, 1.0).unwrap_err();
        assert_eq!(
            err,
            ScoringError::DimMismatch {
                expected: 2,
                got: 1
            }
        );
    }

    #[test]
    fn nonfinite_vectors_rejected() {
        let built = build_pairs("s", &[("q", choice_question())]).unwrap();
        let state = vec![1.0, 0.0];
        let candidates = vec![vec![f32::NAN, 0.0], vec![0.0, 1.0]];
        assert_eq!(
            score_question(&built[0], &state, &candidates, 28.0, 1.0),
            Err(ScoringError::NonFinite)
        );
    }

    #[test]
    fn duplicate_question_ids_rejected() {
        let err =
            build_pairs("s", &[("q", choice_question()), ("q", choice_question())]).unwrap_err();
        assert_eq!(err, QuestionError::DuplicateId);
    }

    #[test]
    fn rank_rejects_empty_and_blank_candidates() {
        let one = system_one();
        assert!(matches!(
            one.rank("ctx", &[], "q", 1.0),
            Err(SystemOneError::Question(QuestionError::NoCandidates))
        ));
        assert!(matches!(
            one.rank("ctx", &["".to_string()], "q", 1.0),
            Err(SystemOneError::Question(QuestionError::BadCandidate))
        ));
    }

    // ---- validation: state/action projection routing ----

    /// Head double that records which projection method served each side.
    /// Delegates the math to [`HashProjection`]; the counters are the test.
    struct SideRecordingHead {
        inner: HashProjection,
        state_calls: std::cell::Cell<usize>,
        action_calls: std::cell::Cell<usize>,
    }

    impl ProjectionHead for SideRecordingHead {
        fn name(&self) -> &str {
            self.inner.name()
        }
        fn generation(&self) -> u64 {
            self.inner.generation()
        }
        fn embed_dim(&self) -> usize {
            self.inner.embed_dim()
        }
        fn projection_dim(&self) -> usize {
            self.inner.projection_dim()
        }
        fn logit_scale(&self) -> f32 {
            self.inner.logit_scale()
        }
        fn project_states(
            &self,
            embeddings: &[Vec<f32>],
        ) -> Result<Vec<Vec<f32>>, ProjectionError> {
            self.state_calls.set(self.state_calls.get() + 1);
            self.inner.project_states(embeddings)
        }
        fn project_actions(
            &self,
            embeddings: &[Vec<f32>],
        ) -> Result<Vec<Vec<f32>>, ProjectionError> {
            self.action_calls.set(self.action_calls.get() + 1);
            self.inner.project_actions(embeddings)
        }
    }

    #[test]
    fn candidates_route_through_project_actions() {
        // CLM projects states and candidates with independent heads; the
        // action namespace must reach `project_actions`, not `project_states`.
        let head = SideRecordingHead {
            inner: HashProjection::new("test-head", 8, 16, 28.0).unwrap(),
            state_calls: std::cell::Cell::new(0),
            action_calls: std::cell::Cell::new(0),
        };
        let arena = VectorArena::new(Budget::Bytes(1 << 20)).unwrap();
        arena.reserve(16, 1.0).unwrap();
        let one = SystemOne::new(FakeEmbedder { dim: 8 }, head, Some(arena));
        one.answer("the build is green", &[("q", choice_question())], 1.0)
            .unwrap();
        assert_eq!(
            one.head.state_calls.get(),
            1,
            "states must use project_states"
        );
        assert_eq!(
            one.head.action_calls.get(),
            1,
            "candidates must use project_actions"
        );
    }

    // ---- adversarial: uncached error preservation ----

    /// Embedder that always fails, to prove the uncached path keeps typed
    /// errors instead of wrapping them in [`CacheError`].
    struct FailingEmbedder;

    impl Embedder for FailingEmbedder {
        fn embed(&self, _texts: &[String]) -> Result<Embedded, EmbedError> {
            Err(EmbedError::transport("encoder down"))
        }
    }

    #[test]
    fn uncached_embed_failure_keeps_typed_error() {
        let head = HashProjection::new("test-head", 8, 16, 28.0).unwrap();
        let one = SystemOne::new(FailingEmbedder, head, None);
        let err = one
            .answer("s", &[("q", choice_question())], 1.0)
            .unwrap_err();
        assert!(
            matches!(err, SystemOneError::Embed(EmbedError::Transport(_))),
            "uncached path must preserve SystemOneError::Embed, got {err}"
        );
    }
}
