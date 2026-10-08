#![forbid(unsafe_code)]

//! Fast System 1 decision layer for phlow.
//!
//! A bounded client for the Jev system-one wire protocol (as served by
//! `laya-serve`): many questions about one shared state, answered in one
//! forward pass, each answer carrying a confidence.
//!
//! ```text
//! QuestionBatch --System1Decider::decide--> AnswerBatch (untrusted)
//!                  |- HttpBackend: POST {endpoint}/v1/systemone
//!                  \- MockBackend: scripted, for tests
//!
//! Decision (Approval) --RiskScorer::route--> Route::AutoApprove | Route::Escalate
//! ```
//!
//! Backends that have a real probability source also answer through
//! [`System1Decider::decide_rich`], returning [`RichAnswerBatch`]
//! (full distributions, validated fail-closed). Backends without one
//! return [`System1Error::DistributionUnavailable`] from that path;
//! no backend fabricates a distribution from scalar confidence.
//!
//! System 1 output is untrusted and uncalibrated. The only consumer here,
//! [`RiskScorer`], fails closed: any error, low confidence, contradiction
//! or ineligible risk class escalates to the existing human path, and no
//! answer can approve anything outside `Observe`/`LocalReversible`.
//!
//! The optional API key comes only from [`API_KEY_ENV`], prints as
//! `[REDACTED]` in every `Debug`, has no `Display`, and never enters a
//! batch, state string or error.

mod backend;
mod clef;
mod config;
mod error;
mod ollama;
mod protocol;
mod risk;

pub use backend::{CONNECT_DEADLINE, HttpBackend, MockBackend, REQUEST_DEADLINE, System1Decider};
pub use clef::{DISTRIBUTION_SUM_TOLERANCE, RichAnswer, RichAnswerBatch};
pub use config::{
    API_KEY_ENV, CONFIG_VALUE_BYTES_MAX, DEFAULT_ENDPOINT, DEFAULT_MODEL, ENDPOINT_ENV, MODEL_ENV,
    System1Config,
};
pub use error::System1Error;
pub use ollama::{
    DEFAULT_OLLAMA_ENDPOINT, OLLAMA_CONNECT_DEADLINE, OLLAMA_REQUEST_DEADLINE, OllamaBackend,
    OllamaConfig, TOP_LOGPROBS, TokenLogprob, distribution_from_top_logprobs,
};
pub use protocol::{
    Answer, AnswerBatch, INSTRUCTIONS_BYTES_MAX, ITEM_BYTES_MAX, ITEMS_MAX, QUESTION_ID_BYTES_MAX,
    QUESTIONS_MAX, Question, QuestionBatch, RESPONSE_BYTES_MAX, STATE_BYTES_MAX,
};
pub use risk::{
    Assessment, CONFIDENCE_MIN, CONSISTENT_ID, CONTEXT_BYTES_MAX, Escalation, FORBIDDEN_ID,
    IRREVERSIBLE_ID, REVERSIBLE_ID, RISK_ID, RISK_MAX, RiskScorer, Route, risk_batch,
};
