//! The [`System1Decider`] contract and its HTTP and scripted backends.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use phlow_llm::REDACTED;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::{Client, Url, redirect};

use crate::clef::RichAnswerBatch;
use crate::config::System1Config;
use crate::error::System1Error;
use crate::protocol::{Answer, AnswerBatch, QuestionBatch, RESPONSE_BYTES_MAX};

/// Total deadline for one System 1 request, connect through last body byte.
/// A fast path that waits longer than this is no longer fast; the caller
/// escalates on [`System1Error::Timeout`].
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
/// Deadline for establishing the TCP/TLS connection.
pub const CONNECT_DEADLINE: Duration = Duration::from_secs(1);

/// Answers a whole [`QuestionBatch`] in one call (one forward pass).
///
/// Implementations may be written as `async fn decide`. A backend is not
/// trusted: answers can be missing, extra, malformed or confidently wrong.
/// Callers that act on answers must first run [`AnswerBatch::validate`]
/// against the same batch; [`crate::RiskScorer`] always does.
pub trait System1Decider: Send + Sync {
    fn decide(
        &self,
        batch: &QuestionBatch,
    ) -> impl Future<Output = Result<AnswerBatch, System1Error>> + Send;

    /// Answer `batch` with full probability distributions preserved
    /// ([`RichAnswerBatch`]), alongside the scalar answers [`decide`]
    /// returns.
    ///
    /// The default is the fail-closed contract of the rich path: a
    /// backend that cannot produce a *real* distribution returns
    /// [`System1Error::DistributionUnavailable`]. A backend must never
    /// expand scalar confidence into a pseudo-distribution and pass it
    /// off as measured. Backends with a genuine distribution source
    /// (Clef wire probabilities, Ollama first-token logprobs) override
    /// this; [`decide`]'s behavior is unchanged either way.
    fn decide_rich(
        &self,
        batch: &QuestionBatch,
    ) -> impl Future<Output = Result<RichAnswerBatch, System1Error>> + Send {
        let _ = batch;
        async {
            Err(System1Error::DistributionUnavailable {
                reason: "backend exposes scalar answers only",
            })
        }
    }
}

/// Speaks the Jev system-one protocol over HTTP to `{endpoint}/v1/systemone`.
///
/// No proxy, no redirects, a total deadline of [`REQUEST_DEADLINE`], and a
/// response body cap of [`RESPONSE_BYTES_MAX`]. Must be polled inside a
/// Tokio runtime (reqwest's async client requirement).
pub struct HttpBackend {
    client: Client,
    url: Url,
    model: String,
    authorization: Option<HeaderValue>,
}

impl fmt::Debug for HttpBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpBackend")
            .field("url", &self.url.as_str())
            .field("model", &self.model)
            .field(
                "authorization",
                &self.authorization.as_ref().map(|_| REDACTED),
            )
            .finish()
    }
}

impl HttpBackend {
    /// Validate `config` and build the client. The key, if any, becomes a
    /// sensitive header value and is not retained anywhere else.
    pub fn new(config: &System1Config) -> Result<Self, System1Error> {
        let url = config.request_url()?;
        let authorization = match config.api_key() {
            None => None,
            Some(key) => {
                let mut value = HeaderValue::from_str(&format!("Bearer {}", key.expose()))
                    .map_err(|_| System1Error::Config {
                        reason: "api key is not a valid header value",
                    })?;
                value.set_sensitive(true);
                Some(value)
            }
        };
        let client = Client::builder()
            .timeout(REQUEST_DEADLINE)
            .connect_timeout(CONNECT_DEADLINE)
            .redirect(redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| System1Error::Config {
                reason: "http client could not be built",
            })?;
        Ok(HttpBackend {
            client,
            url,
            model: config.model.clone(),
            authorization,
        })
    }
}

impl System1Decider for HttpBackend {
    async fn decide(&self, batch: &QuestionBatch) -> Result<AnswerBatch, System1Error> {
        let body = batch.to_wire(&self.model)?;
        let mut request = self
            .client
            .post(self.url.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(authorization) = &self.authorization {
            request = request.header(AUTHORIZATION, authorization.clone());
        }
        let mut response = request.send().await.map_err(transport_error)?;
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
        AnswerBatch::from_wire(batch, &received)
    }
}

fn transport_error(error: reqwest::Error) -> System1Error {
    if error.is_timeout() {
        return System1Error::Timeout;
    }
    System1Error::transport(&error.to_string())
}

/// Deterministic scripted backend for tests.
///
/// Returns every scripted answer on each call, keyed by question id, even
/// ids the batch never asked, so tests can script missing, extra, wrong-kind
/// and high-confidence wrong answers. Or it fails every call with one
/// scripted error. Records the call count and the last batch it received.
#[derive(Debug, Default)]
pub struct MockBackend {
    script: BTreeMap<String, Answer>,
    failure: Option<System1Error>,
    calls: AtomicUsize,
    last_batch: Mutex<Option<QuestionBatch>>,
}

impl MockBackend {
    /// A backend with an empty script: every call returns no answers.
    pub fn new() -> Self {
        MockBackend::default()
    }

    /// Script `answer` for question `id`, replacing any earlier script for it.
    pub fn with_answer(mut self, id: &str, answer: Answer) -> Self {
        self.script.insert(id.to_owned(), answer);
        self
    }

    /// A backend whose every call fails with `error`.
    pub fn failing(error: System1Error) -> Self {
        MockBackend {
            failure: Some(error),
            ..MockBackend::default()
        }
    }

    /// How many times `decide` was called, including rejected batches.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    /// A copy of the last batch `decide` received, if any.
    pub fn last_batch(&self) -> Option<QuestionBatch> {
        self.last_batch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl System1Decider for MockBackend {
    async fn decide(&self, batch: &QuestionBatch) -> Result<AnswerBatch, System1Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self
            .last_batch
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(batch.clone());
        batch.validate()?;
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        Ok(AnswerBatch {
            answers: self.script.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Question;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    fn ready<F: Future>(future: F) -> F::Output {
        let mut future = pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("future was expected to be ready"),
        }
    }

    fn noul_batch() -> QuestionBatch {
        let mut questions = BTreeMap::new();
        questions.insert(
            "urgent".to_owned(),
            Question::Noul {
                instructions: "Is it urgent?".to_owned(),
            },
        );
        QuestionBatch {
            state: "test".to_owned(),
            questions,
        }
    }

    // Validation: the scalar path is unchanged by the rich path.
    #[test]
    fn mock_scalar_decide_still_answers() {
        let backend = MockBackend::new().with_answer(
            "urgent",
            Answer::Noul {
                yes: true,
                probability: 0.9,
            },
        );
        let batch = noul_batch();
        let answers = ready(backend.decide(&batch)).unwrap();
        assert_eq!(
            answers.answers["urgent"],
            Answer::Noul {
                yes: true,
                probability: 0.9
            }
        );
    }

    // Adversarial: a scalar-only backend must not fabricate a
    // distribution; the rich path fails closed with the typed error.
    #[test]
    fn mock_decide_rich_is_distribution_unavailable() {
        let backend = MockBackend::new().with_answer(
            "urgent",
            Answer::Noul {
                yes: true,
                probability: 0.9,
            },
        );
        let batch = noul_batch();
        let result = ready(backend.decide_rich(&batch));
        assert!(matches!(
            result,
            Err(System1Error::DistributionUnavailable { .. })
        ));
    }
}
