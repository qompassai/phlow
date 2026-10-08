//! Ollama-backed System 1 decider with real answer distributions.
//!
//! Ollama serves an OpenAI-compatible chat endpoint at
//! `{endpoint}/v1/chat/completions`. With `logprobs` requested, each
//! generated token carries `top_logprobs`: the model's probability
//! distribution over the vocabulary at that token position. That is a
//! real distribution from the serving model — the only honest source
//! this backend has — so the rich answer path is built from it and
//! from nothing else.
//!
//! # The derivation, stated exactly
//!
//! For a Choice question the backend asks the model to reply with
//! exactly one option word and reads the FIRST generated token's
//! `top_logprobs`. The first-token distribution is a distribution over
//! vocabulary tokens; each option corresponds to the set of token
//! surface forms whose trimmed, lowercased text equals the option's
//! trimmed, lowercased text (e.g. `Positive`, `positive` and
//! ` Positive` are the same option). The option masses are those sets'
//! total probability, renormalized over the option set: the result is
//! exactly the model's first-token probability distribution
//! conditioned on the answer being one of the options, which is what
//! the returned `distribution` claims to be — no more, no less. It is
//! a *derived* distribution and is labeled as such wherever it is
//! described.
//!
//! Fail-closed rules, all typed errors, never a fabricated vector:
//!
//! - an option with no matching token in `top_logprobs` has unknown
//!   probability (it may sit below the top-k cut), so the question
//!   fails with a protocol error instead of assigning it zero;
//! - a response without `logprobs` yields
//!   [`System1Error::DistributionUnavailable`];
//! - Score questions have no per-level distribution in a chat
//!   completion's token logprobs, so they fail with
//!   [`System1Error::DistributionUnavailable`];
//! - non-finite logprobs, empty `top_logprobs`, or a response that
//!   does not parse are protocol errors.
//!
//! Noul questions are the two-option case over `no`/`yes`, giving
//! `distribution = [P(no), P(yes)]` under the same conditioning.
//!
//! # Why this backend is synchronous
//!
//! The workspace's only async task machinery is the msgpack
//! transport's single worker (a standing architectural invariant the
//! gauntlet verifies by source scan). A local-daemon backend has no
//! place in that runtime, and its natural consumers — the canary
//! battery's synchronous `ProbeBackend` seam, CLIs, tests — are
//! blocking callers. [`OllamaBackend`] therefore does bounded
//! blocking I/O on the caller's thread and deliberately does not
//! implement the async [`crate::System1Decider`] trait.

use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Url, redirect};
use serde::Deserialize;

use crate::clef::{RichAnswer, RichAnswerBatch};
use crate::error::System1Error;
use crate::protocol::{Answer, AnswerBatch, Question, QuestionBatch, RESPONSE_BYTES_MAX};

/// Default Ollama endpoint (loopback; the daemon phlow develops against).
pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://127.0.0.1:11434";
/// Total deadline for one per-question chat request. Larger than the
/// Jev path's deadline because a cold model load is part of a first
/// request to a local daemon; the bound still exists and is named.
pub const OLLAMA_REQUEST_DEADLINE: Duration = Duration::from_secs(120);
/// Deadline for establishing the TCP connection.
pub const OLLAMA_CONNECT_DEADLINE: Duration = Duration::from_secs(5);
/// Alternatives requested per token position. 20 is the maximum the
/// OpenAI-compatible endpoint accepts (verified against the live
/// daemon, Ollama 0.40, 2026-10-08).
pub const TOP_LOGPROBS: u8 = 20;

/// Configuration for [`OllamaBackend`]: daemon endpoint and the exact
/// model tag to ask for. Both are operator-supplied and validated at
/// construction; nothing is read from the environment here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaConfig {
    /// Base endpoint, e.g. `http://127.0.0.1:11434`.
    pub endpoint: String,
    /// Model tag as the daemon names it, e.g. `qwen2.5-coder:7b`.
    pub model: String,
}

impl OllamaConfig {
    /// A config for `model` on the default loopback endpoint.
    pub fn new(model: &str) -> Self {
        OllamaConfig {
            endpoint: DEFAULT_OLLAMA_ENDPOINT.to_owned(),
            model: model.to_owned(),
        }
    }

    /// The chat-completions URL for this config.
    fn chat_url(&self) -> Result<Url, System1Error> {
        if self.model.is_empty() {
            return Err(System1Error::Config {
                reason: "ollama model must not be empty",
            });
        }
        let base = self.endpoint.trim_end_matches('/');
        Url::parse(&format!("{base}/v1/chat/completions")).map_err(|_| System1Error::Config {
            reason: "ollama endpoint is not a valid URL",
        })
    }
}

/// A synchronous System 1 backend served by a local Ollama daemon,
/// deriving rich answers from first-token logprobs (see the module
/// docs for the exact derivation and the fail-closed rules).
pub struct OllamaBackend {
    client: Client,
    url: Url,
    model: String,
}

impl std::fmt::Debug for OllamaBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaBackend")
            .field("url", &self.url.as_str())
            .field("model", &self.model)
            .finish()
    }
}

impl OllamaBackend {
    /// Validate `config` and build the client. No proxy, no redirects,
    /// a per-request deadline of [`OLLAMA_REQUEST_DEADLINE`], and a
    /// response body cap of [`RESPONSE_BYTES_MAX`].
    pub fn new(config: &OllamaConfig) -> Result<Self, System1Error> {
        let url = config.chat_url()?;
        let client = Client::builder()
            .timeout(OLLAMA_REQUEST_DEADLINE)
            .connect_timeout(OLLAMA_CONNECT_DEADLINE)
            .redirect(redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| System1Error::Config {
                reason: "http client could not be built",
            })?;
        Ok(OllamaBackend {
            client,
            url,
            model: config.model.clone(),
        })
    }

    /// Ask the daemon one question and derive its rich answer from the
    /// first generated token's `top_logprobs`. Blocking I/O, bounded
    /// by [`OLLAMA_REQUEST_DEADLINE`].
    fn ask_one(&self, state: &str, question: &Question) -> Result<RichAnswer, System1Error> {
        let (instructions, options): (&str, Vec<String>) = match question {
            Question::Choice {
                instructions,
                options,
            } => (instructions, options.clone()),
            Question::Noul { instructions } => {
                (instructions, vec!["no".to_owned(), "yes".to_owned()])
            }
            Question::Score { .. } => {
                return Err(System1Error::DistributionUnavailable {
                    reason: "ollama logprobs expose no per-level score distribution",
                });
            }
        };
        let prompt = format!(
            "{instructions}\nAnswer with exactly one of these options: {}. \
             Reply with the option word only.",
            options.join(", ")
        );
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": state },
                { "role": "user", "content": prompt },
            ],
            "logprobs": true,
            "top_logprobs": TOP_LOGPROBS,
            "temperature": 0,
            "max_tokens": 1,
        });
        let response = self
            .client
            .post(self.url.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(&body).map_err(|e| System1Error::protocol(&e.to_string()))?)
            .send()
            .map_err(transport_error)?;
        let status = response.status();
        if !status.is_success() {
            let detail = format!("ollama returned HTTP {}", status.as_u16());
            return Err(System1Error::transport(&detail));
        }
        let bytes = response.bytes().map_err(transport_error)?;
        if bytes.len() > RESPONSE_BYTES_MAX {
            return Err(System1Error::protocol(
                "ollama response exceeds RESPONSE_BYTES_MAX",
            ));
        }
        let top = first_token_top_logprobs(&bytes)?;
        let distribution = distribution_from_top_logprobs(&options, &top)?;
        let rich = match question {
            Question::Noul { .. } => {
                // Options are ["no", "yes"]: distribution is [P(no), P(yes)].
                let probability = distribution[1];
                RichAnswer {
                    answer: Answer::Noul {
                        yes: probability > 0.5,
                        probability,
                    },
                    distribution,
                }
            }
            _ => {
                let selected = argmax_first(&distribution);
                RichAnswer {
                    answer: Answer::Choice {
                        selected,
                        probability: distribution[selected],
                    },
                    distribution,
                }
            }
        };
        rich.validate(question)?;
        Ok(rich)
    }
}

impl OllamaBackend {
    /// Answer `batch` with full distributions preserved: one bounded
    /// blocking chat request per question, each validated fail-closed
    /// (see the module docs for the derivation and the error rules).
    pub fn decide_rich(&self, batch: &QuestionBatch) -> Result<RichAnswerBatch, System1Error> {
        batch.validate()?;
        let mut answers = std::collections::BTreeMap::new();
        for (id, question) in &batch.questions {
            let rich = self.ask_one(&batch.state, question)?;
            answers.insert(id.clone(), rich);
        }
        let rich = RichAnswerBatch { answers };
        rich.validate(batch)?;
        Ok(rich)
    }

    /// The scalar answers for `batch`: a collapse of
    /// [`decide_rich`](Self::decide_rich) (top answer plus its
    /// probability), never a separate, weaker source.
    pub fn decide(&self, batch: &QuestionBatch) -> Result<AnswerBatch, System1Error> {
        Ok(self.decide_rich(batch)?.to_answer_batch())
    }
}

fn transport_error(error: reqwest::Error) -> System1Error {
    if error.is_timeout() {
        return System1Error::Timeout;
    }
    System1Error::transport(&error.to_string())
}

/// Index of the first maximum (Python `max` tie behavior, matching
/// the rest of the codebase).
fn argmax_first(distribution: &[f64]) -> usize {
    let mut best = 0_usize;
    for (index, value) in distribution.iter().enumerate() {
        if *value > distribution[best] {
            best = index;
        }
    }
    best
}

/// One token alternative from a chat completion's `top_logprobs`.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenLogprob {
    /// The token's surface text, exactly as the daemon returned it.
    pub token: String,
    /// Natural log of the token's probability at that position.
    pub logprob: f64,
}

// OpenAI-compatible chat completion wire shapes. Not closed: the
// daemon adds fields (usage, fingerprints) that are not ours to police.

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    logprobs: Option<ChatLogprobs>,
}

#[derive(Deserialize)]
struct ChatLogprobs {
    content: Vec<ChatToken>,
}

#[derive(Deserialize)]
struct ChatToken {
    top_logprobs: Vec<TokenLogprobWire>,
}

#[derive(Deserialize)]
struct TokenLogprobWire {
    token: String,
    logprob: f64,
}

/// Extract the first generated token's alternatives from a chat
/// completion body. A body without logprobs is
/// [`System1Error::DistributionUnavailable`]: the daemon was asked
/// for them and did not provide them, so no distribution exists.
fn first_token_top_logprobs(body: &[u8]) -> Result<Vec<TokenLogprob>, System1Error> {
    let parsed: ChatResponse =
        serde_json::from_slice(body).map_err(|e| System1Error::protocol(&e.to_string()))?;
    let Some(choice) = parsed.choices.into_iter().next() else {
        return Err(System1Error::protocol("chat response has no choices"));
    };
    let Some(logprobs) = choice.logprobs else {
        return Err(System1Error::DistributionUnavailable {
            reason: "chat response carries no logprobs",
        });
    };
    let Some(first) = logprobs.content.into_iter().next() else {
        return Err(System1Error::protocol(
            "chat response logprobs have no token entries",
        ));
    };
    if first.top_logprobs.is_empty() {
        return Err(System1Error::protocol("first token has no top_logprobs"));
    }
    Ok(first
        .top_logprobs
        .into_iter()
        .map(|wire| TokenLogprob {
            token: wire.token,
            logprob: wire.logprob,
        })
        .collect())
}

/// Normalize an option or token surface form for matching: surrounding
/// whitespace is token-boundary framing, and case is not a different
/// option.
fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Derive the option distribution from first-token `top_logprobs`:
/// group token surface forms by normalized option text, sum their
/// probabilities, and renormalize over the option set (the conditional
/// distribution described in the module docs).
///
/// Fail-closed: duplicate options after normalization, a non-finite
/// logprob, or any option with zero matched mass is a protocol error
/// — an unmatched option's true probability is unknown, not zero.
pub fn distribution_from_top_logprobs(
    options: &[String],
    top: &[TokenLogprob],
) -> Result<Vec<f64>, System1Error> {
    let protocol = |msg: &str| System1Error::protocol(msg);
    if options.is_empty() || top.is_empty() {
        return Err(protocol("no options or no top_logprobs to derive from"));
    }
    let mut normalized_options = Vec::with_capacity(options.len());
    for option in options {
        let normalized = normalize(option);
        if normalized.is_empty() || normalized_options.contains(&normalized) {
            return Err(protocol("options collide after normalization"));
        }
        normalized_options.push(normalized);
    }
    let mut masses = vec![0.0_f64; options.len()];
    for entry in top {
        if !entry.logprob.is_finite() {
            return Err(protocol("top_logprobs carry a non-finite logprob"));
        }
        let normalized = normalize(&entry.token);
        if let Some(index) = normalized_options.iter().position(|o| *o == normalized) {
            masses[index] += entry.logprob.exp();
        }
    }
    let mut total = 0.0_f64;
    for mass in &masses {
        if *mass <= 0.0 {
            return Err(protocol(
                "an option is absent from top_logprobs; its probability is unknown",
            ));
        }
        total += mass;
    }
    if !total.is_finite() || total <= 0.0 {
        return Err(protocol("option masses do not form a distribution"));
    }
    Ok(masses.iter().map(|mass| mass / total).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(pairs: &[(&str, f64)]) -> Vec<TokenLogprob> {
        pairs
            .iter()
            .map(|(token, logprob)| TokenLogprob {
                token: (*token).to_owned(),
                logprob: *logprob,
            })
            .collect()
    }

    fn options(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    // --- Validation: the derivation does what it claims ---

    #[test]
    fn groups_surface_variants_and_renormalizes() {
        // "Positive"/"positive" are one option; an unrelated token is
        // conditioned away by the renormalization.
        let dist = distribution_from_top_logprobs(
            &options(&["negative", "positive"]),
            &top(&[
                ("Positive", -0.7),
                ("positive", -1.4),
                ("Negative", -2.0),
                ("The", -3.0),
            ]),
        )
        .unwrap();
        let positive = (-0.7_f64).exp() + (-1.4_f64).exp();
        let negative = (-2.0_f64).exp();
        assert!((dist[1] - positive / (positive + negative)).abs() < 1e-12);
        assert!((dist[0] - negative / (positive + negative)).abs() < 1e-12);
        assert!((dist.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn extracts_first_token_top_logprobs() {
        let body = br#"{"choices":[{"message":{"content":"Positive"},"logprobs":{"content":[{"token":"Positive","logprob":-0.5,"top_logprobs":[{"token":"Positive","logprob":-0.5},{"token":"Negative","logprob":-1.5}]}]}}]}"#;
        let top = first_token_top_logprobs(body).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].token, "Positive");
    }

    // --- Adversarial: anything unusable fails closed ---

    #[test]
    fn missing_option_mass_is_an_error_not_zero() {
        let result = distribution_from_top_logprobs(
            &options(&["approve", "escalate"]),
            &top(&[("approve", -0.1), ("Approve", -2.0)]),
        );
        assert!(matches!(result, Err(System1Error::Protocol { .. })));
    }

    #[test]
    fn non_finite_logprob_is_an_error() {
        let result = distribution_from_top_logprobs(
            &options(&["yes", "no"]),
            &top(&[("yes", f64::NAN), ("no", -1.0)]),
        );
        assert!(result.is_err());
    }

    #[test]
    fn empty_top_logprobs_is_an_error() {
        let result = distribution_from_top_logprobs(&options(&["yes", "no"]), &[]);
        assert!(result.is_err());
    }

    #[test]
    fn colliding_options_are_an_error() {
        let result =
            distribution_from_top_logprobs(&options(&["Yes", "yes"]), &top(&[("yes", -0.5)]));
        assert!(result.is_err());
    }

    #[test]
    fn response_without_logprobs_is_distribution_unavailable() {
        let body = br#"{"choices":[{"message":{"content":"Positive"},"logprobs":null}]}"#;
        let result = first_token_top_logprobs(body);
        assert!(matches!(
            result,
            Err(System1Error::DistributionUnavailable { .. })
        ));
    }

    #[test]
    fn malformed_response_is_a_protocol_error() {
        let result = first_token_top_logprobs(b"not json");
        assert!(matches!(result, Err(System1Error::Protocol { .. })));
    }
}
