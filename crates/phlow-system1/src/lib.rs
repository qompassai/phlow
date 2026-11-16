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
//!                  |- KevBackend: POST {endpoint}/v1/systemone (llama.cpp, Kev-4B)
//!                  \- MockBackend: scripted, for tests
//!
//! Decision (Approval) --RiskScorer::route--> Route::AutoApprove | Route::Escalate
//! QuestionBatch --TriageScorer::route--> TriageVerdict::Accepted | ::Escalated
//! ```
//!
//! Backends that have a real probability source also answer through
//! [`System1Decider::decide_rich`], returning [`RichAnswerBatch`]
//! (full distributions, validated fail-closed). Backends without one
//! return [`System1Error::DistributionUnavailable`] from that path;
//! no backend fabricates a distribution from scalar confidence.
//!
//! System 1 output is untrusted and uncalibrated. Both consumers here
//! fail closed. [`RiskScorer`]: any error, low confidence, contradiction
//! or ineligible risk class escalates to the existing human path, and no
//! answer can approve anything outside `Observe`/`LocalReversible`.
//! [`TriageScorer`] (the D-1622 routing/triage gate over [`KevBackend`]):
//! any error or below-cutoff confidence escalates the item to the
//! caller's existing path, and the scorer holds no policy capability —
//! a triage verdict is data for the caller's path, never an action.
//!
//! The optional API key comes only from [`API_KEY_ENV`], prints as
//! `[REDACTED]` in every `Debug`, has no `Display`, and never enters a
//! batch, state string or error.

mod backend;
mod clef;
mod config;
mod error;
mod kev;
mod ollama;
mod protocol;
mod risk;
mod triage;

pub use backend::{CONNECT_DEADLINE, HttpBackend, MockBackend, REQUEST_DEADLINE, System1Decider};
pub use clef::{DISTRIBUTION_SUM_TOLERANCE, RichAnswer, RichAnswerBatch};
pub use config::{
    API_KEY_ENV, CONFIG_VALUE_BYTES_MAX, DEFAULT_ENDPOINT, DEFAULT_MODEL, ENDPOINT_ENV, MODEL_ENV,
    System1Config,
};
pub use error::System1Error;
pub use kev::{
    DEFAULT_KEV_ENDPOINT, KEV_SCORE_LEVELS_MAX, KEV_SCORE_LEVELS_MIN, KevBackend, KevConfig,
};
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
pub use triage::{
    TRIAGE_CONFIDENCE_CUTOFF, TriageEscalation, TriageOutcome, TriageScorer, TriageVerdict,
};
