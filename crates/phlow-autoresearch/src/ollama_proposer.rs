//! The live proposer binding: the planner specialist, behind the
//! [`Proposer`] trait, over the Ollama-shaped HTTP API.
//!
//! The planner emits one change-set as strict JSON. Parsing follows
//! phlow's reviewer-verdict discipline: exact shape (unknown fields
//! rejected), exactly one bounded repair normalization — a single
//! surrounding ```` ``` ```` fence is stripped — and never free-form
//! trust. Bounds from [`crate::changeset`] are re-checked here so a
//! malformed proposal fails as [`ProposeError::Invalid`] before the
//! loop ever sees it; containment validation stays the loop's job.
//!
//! Transport is a trait ([`ChatTransport`]) so unit tests run against
//! a fake and never touch a live daemon. The production transport is
//! a hand-rolled HTTP/1.1 client over `std::net::TcpStream` — no new
//! dependencies — that refuses non-loopback endpoints at construction:
//! the loop's only network target is the local model server, and this
//! binding has no opt-in field a change-set could flip. Endpoint
//! configuration follows the design's rose note: `ROSE_HOST`-style
//! naming (rose's `ROSE_*`-in-lieu-of-`OLLAMA_*` identity ruling),
//! loopback Ollama as the default because the specialists are served
//! by stock Ollama today. No secrets exist on this path — loopback
//! serving is unauthenticated — and none are accepted.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::Value;

use crate::changeset::{
    CHANGE_SET_ID_CHARS_MAX, CHANGE_SET_PATHS_MAX, CHANGE_SET_PAYLOAD_BYTES_MAX,
    CHANGE_SET_RATIONALE_CHARS_MAX, ChangeKind, ChangeSet,
};
use crate::proposer::{ProposeContext, ProposeError, Proposer};

/// Maximum bytes in one planner response the binding will parse.
pub const PLANNER_RESPONSE_BYTES_MAX: usize = 64 * 1024;
/// Maximum bytes in one HTTP response head (status line + headers).
pub const HTTP_HEAD_BYTES_MAX: usize = 16 * 1024;
/// TCP connect deadline for the model server.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Read/write deadline once connected. A 30B planner generating a
/// bounded JSON proposal is slow but not unbounded.
pub const IO_TIMEOUT: Duration = Duration::from_secs(120);
/// Default serving endpoint: stock Ollama on loopback, where the
/// specialists are bound today.
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";
/// Default planner specialist model (the `[specialists]` planner
/// binding).
pub const DEFAULT_PLANNER_MODEL: &str = "hf-nemotron-3.5-lightning-30b";
/// Maximum characters of transport detail kept in an error message.
const ERROR_DETAIL_CHARS_MAX: usize = 256;

/// One completion call against a model server. Implementations must
/// bound the response they return; errors are bounded human-readable
/// detail, mapped to [`ProposeError::Invalid`] by the proposer.
pub trait ChatTransport: std::fmt::Debug {
    /// Generate a completion for `prompt` from `model`.
    ///
    /// # Errors
    /// A bounded message for every failure: unreachable daemon,
    /// non-200 status, malformed envelope, oversized body.
    fn generate(&self, model: &str, prompt: &str) -> Result<String, String>;
}

/// Endpoint + model configuration for the proposer binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposerConfig {
    /// Base URL of the Ollama-shaped server, loopback only.
    pub base_url: String,
    /// Planner model name as served.
    pub model: String,
}

impl Default for ProposerConfig {
    fn default() -> Self {
        ProposerConfig {
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_PLANNER_MODEL.to_string(),
        }
    }
}

impl ProposerConfig {
    /// Configuration from the environment, rose-style: `ROSE_HOST`
    /// (a `host:port` or full `http://` URL) overrides the endpoint,
    /// `ROSE_AUTORESEARCH_MODEL` overrides the planner model. Unset
    /// or empty variables fall back to the defaults; no other
    /// variables are read, and no secret is among them.
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = ProposerConfig::default();
        if let Ok(host) = std::env::var("ROSE_HOST") {
            let host = host.trim();
            if !host.is_empty() {
                config.base_url = if host.contains("://") {
                    host.to_string()
                } else {
                    format!("http://{host}")
                };
            }
        }
        if let Ok(model) = std::env::var("ROSE_AUTORESEARCH_MODEL")
            && !model.trim().is_empty()
        {
            config.model = model;
        }
        config
    }
}

/// Production [`ChatTransport`]: HTTP/1.1 POST to `/api/generate`.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    host: String,
    port: u16,
}

impl HttpTransport {
    /// Parse and validate a base URL. Only `http://` on a loopback
    /// host is accepted — anything else is a construction error, so a
    /// misconfigured endpoint fails before the loop starts rather
    /// than mid-run.
    ///
    /// # Errors
    /// A bounded message naming the defect.
    pub fn new(base_url: &str) -> Result<Self, String> {
        let authority = base_url
            .strip_prefix("http://")
            .ok_or_else(|| format!("endpoint {base_url:?} is not an http:// URL"))?;
        let authority = authority.trim_end_matches('/');
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (
                host.trim_matches(['[', ']']).to_string(),
                port.parse::<u16>()
                    .map_err(|_| format!("endpoint {base_url:?} has an unusable port"))?,
            ),
            None => (authority.trim_matches(['[', ']']).to_string(), 11434),
        };
        if !is_loopback_host(&host) {
            return Err(format!(
                "endpoint host {host:?} is not loopback; the proposer binding is loopback-only"
            ));
        }
        Ok(HttpTransport { host, port })
    }
}

/// Whether a host literal names this machine.
fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

impl ChatTransport for HttpTransport {
    fn generate(&self, model: &str, prompt: &str) -> Result<String, String> {
        let address = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|err| format!("cannot resolve endpoint: {err}"))?
            .next()
            .ok_or_else(|| "endpoint resolved to no address".to_string())?;
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
            .map_err(|err| format!("cannot reach model server at {address}: {err}"))?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|err| err.to_string())?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|err| err.to_string())?;
        let body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "format": "json",
            // The binding consumes the answer channel only; a
            // thinking planner's trace is not part of the contract,
            // and with stream:false the daemon sends nothing until
            // generation ends — an unbounded trace is a read timeout
            // (observed live against the 30B planner). Models that
            // do not think ignore the flag.
            "think": false,
        })
        .to_string();
        let request = format!(
            "POST /api/generate HTTP/1.1\r\nhost: {}\r\ncontent-type: application/json\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n{}",
            address,
            body.len(),
            body
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|err| format!("cannot send request: {err}"))?;
        let mut raw = Vec::new();
        stream
            .take((HTTP_HEAD_BYTES_MAX + PLANNER_RESPONSE_BYTES_MAX + 1) as u64)
            .read_to_end(&mut raw)
            .map_err(|err| format!("cannot read response: {err}"))?;
        parse_generate_response(&raw)
    }
}

/// Split an HTTP response into status + body and extract the
/// `response` field of Ollama's generate envelope.
fn parse_generate_response(raw: &[u8]) -> Result<String, String> {
    let head_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "response has no header terminator".to_string())?;
    if head_end > HTTP_HEAD_BYTES_MAX {
        return Err("response head exceeds the byte bound".to_string());
    }
    let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
    let status: u32 = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| "response status line is malformed".to_string())?;
    if status != 200 {
        return Err(format!("model server returned HTTP {status}"));
    }
    let body = &raw[head_end + 4..];
    // Ollama's Go server answers with chunked transfer encoding
    // whenever the envelope outgrows its write buffer (the generate
    // envelope carries the full token `context`, so any realistic
    // prompt qualifies). Decode before the byte bound and the JSON
    // parse, or the chunk framing corrupts both. Found live: the
    // first run-live attempt crashed every proposal on this.
    let decoded;
    let body: &[u8] = if head.to_lowercase().contains("transfer-encoding: chunked") {
        decoded = decode_chunked(body)?;
        &decoded
    } else {
        body
    };
    if body.len() > PLANNER_RESPONSE_BYTES_MAX {
        return Err("response body exceeds the byte bound".to_string());
    }
    let envelope: Value = serde_json::from_slice(body)
        .map_err(|err| format!("response envelope is malformed JSON: {err}"))?;
    envelope
        .get("response")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "response envelope has no response string".to_string())
}

/// Decode an HTTP/1.1 chunked transfer-encoded body: hex size
/// lines (extensions after `;` ignored), each followed by exactly
/// that many bytes and a CRLF; a zero chunk ends the body and any
/// trailers are ignored. The decoded length is bounded by
/// [`PLANNER_RESPONSE_BYTES_MAX`] as it accumulates. Every framing
/// defect fails closed.
fn decode_chunked(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    let mut rest = body;
    loop {
        let line_end = rest
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| "chunked body has no chunk-size line".to_string())?;
        let size_text = String::from_utf8_lossy(&rest[..line_end]).into_owned();
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| format!("chunk size {size_text:?} is not hex"))?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(decoded);
        }
        if rest.len() < size + 2 {
            return Err("chunked body ends inside a chunk".to_string());
        }
        decoded.extend_from_slice(&rest[..size]);
        if decoded.len() > PLANNER_RESPONSE_BYTES_MAX {
            return Err("response body exceeds the byte bound".to_string());
        }
        if &rest[size..size + 2] != b"\r\n" {
            return Err("chunk is not CRLF-terminated".to_string());
        }
        rest = &rest[size + 2..];
    }
}

/// The planner specialist as a [`Proposer`].
#[derive(Debug)]
pub struct OllamaProposer {
    model: String,
    transport: Box<dyn ChatTransport>,
}

impl OllamaProposer {
    /// A proposer over an explicit transport (tests inject a fake).
    #[must_use]
    pub fn new(model: impl Into<String>, transport: Box<dyn ChatTransport>) -> Self {
        OllamaProposer {
            model: model.into(),
            transport,
        }
    }

    /// A proposer from configuration, over the HTTP transport.
    ///
    /// # Errors
    /// [`ProposeError::Invalid`] when the configured endpoint is not
    /// a loopback `http://` URL.
    pub fn from_config(config: &ProposerConfig) -> Result<Self, ProposeError> {
        let transport = HttpTransport::new(&config.base_url).map_err(ProposeError::Invalid)?;
        Ok(OllamaProposer::new(
            config.model.clone(),
            Box::new(transport),
        ))
    }
}

impl Proposer for OllamaProposer {
    fn propose(&mut self, context: &ProposeContext) -> Result<ChangeSet, ProposeError> {
        let prompt = build_prompt(context);
        let response = self
            .transport
            .generate(&self.model, &prompt)
            .map_err(|detail| {
                ProposeError::Invalid(format!(
                    "planner transport: {}",
                    truncate(&detail, ERROR_DETAIL_CHARS_MAX)
                ))
            })?;
        parse_change_set(&response)
    }
}

/// Truncate `text` to `max` characters on a char boundary.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

/// The planner prompt: role, run state, and the exact output shape.
/// The model is told the bounds it will be held to; the binding still
/// enforces them, because a prompt is not a contract.
fn build_prompt(context: &ProposeContext) -> String {
    let incumbent = context
        .incumbent_metric
        .map(|metric| format!("{metric:.4}"))
        .unwrap_or_else(|| "none yet (the first proposal is the baseline probe)".to_string());
    let head = context.ledger_head_sha256.as_deref().unwrap_or("none");
    format!(
        "You are the planner in a bounded autoresearch loop over phlow's trainlab. \
         Propose exactly one experiment as a single JSON object and nothing else.\n\
         Iteration: {}\nIncumbent dev-split mean pass@1: {incumbent}\nLedger head: {head}\n\
         Output shape (exact keys, no extras):\n\
         {{\"id\": \"<short slug, [A-Za-z0-9._-], at most {id_max} chars>\", \
         \"kind\": \"trainlab_config\" or \"file_patch\", \
         \"paths\": [<worktree-relative paths, file_patch only>], \
         \"payload\": \"<config delta JSON or unified patch, at most {payload_max} bytes>\", \
         \"rationale\": \"<one line, at most {rationale_max} chars>\"}}\n\
         Gate rules, enforced exactly (a violation is discarded unmeasured):\n\
         - kind trainlab_config: paths MUST be [] — a config change-set carries \
         no paths. payload is a JSON object holding any of these run-configuration \
         fields (unknown fields are rejected): temperature (number, 0.0 to 2.0), \
         groups, group_size, per_family, seed (integers), families (array of \
         task-family names), invalid_penalty (number).\n\
         - kind file_patch: paths lists the worktree files the patch touches and \
         payload is a unified diff against them. The worktree holds exactly one \
         file, base-config.json — a JSON object with the same run-configuration \
         fields — which a patch may edit to move the base configuration.\n\
         One idea per proposal. A gain below 0.01 pass@1 is discarded as noise, \
         so propose changes with a plausible effect larger than that.",
        context.iteration,
        id_max = CHANGE_SET_ID_CHARS_MAX,
        payload_max = CHANGE_SET_PAYLOAD_BYTES_MAX,
        rationale_max = CHANGE_SET_RATIONALE_CHARS_MAX,
    )
}

/// Parse one planner response into a change-set: strip at most one
/// surrounding code fence (the single bounded normalization), then
/// require the exact shape and re-check every bound.
fn parse_change_set(response: &str) -> Result<ChangeSet, ProposeError> {
    if response.len() > PLANNER_RESPONSE_BYTES_MAX {
        return Err(ProposeError::Invalid(
            "planner response exceeds the byte bound".to_string(),
        ));
    }
    let text = strip_one_fence(response.trim());
    let value: Value = serde_json::from_str(text)
        .map_err(|err| ProposeError::Invalid(format!("planner output is not JSON: {err}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| ProposeError::Invalid("planner output is not a JSON object".to_string()))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "id" | "kind" | "paths" | "payload" | "rationale"
        ) {
            return Err(ProposeError::Invalid(format!(
                "planner output carries unexpected key {key:?}"
            )));
        }
    }
    let field = |name: &str| {
        object.get(name).and_then(Value::as_str).ok_or_else(|| {
            ProposeError::Invalid(format!("planner output lacks string field {name}"))
        })
    };
    let kind = match field("kind")? {
        "trainlab_config" => ChangeKind::TrainlabConfig,
        "file_patch" => ChangeKind::FilePatch,
        other => {
            return Err(ProposeError::Invalid(format!(
                "planner output has unknown kind {other:?}"
            )));
        }
    };
    let paths = match object.get("paths") {
        None => Vec::new(),
        Some(Value::Array(entries)) => {
            if entries.len() > CHANGE_SET_PATHS_MAX {
                return Err(ProposeError::Invalid(
                    "planner output lists too many paths".to_string(),
                ));
            }
            entries
                .iter()
                .map(|entry| {
                    entry.as_str().map(str::to_string).ok_or_else(|| {
                        ProposeError::Invalid("planner path is not a string".to_string())
                    })
                })
                .collect::<Result<Vec<String>, ProposeError>>()?
        }
        Some(_) => {
            return Err(ProposeError::Invalid(
                "planner paths field is not an array".to_string(),
            ));
        }
    };
    let change_set = ChangeSet {
        id: field("id")?.to_string(),
        kind,
        paths,
        payload: field("payload")?.to_string(),
        rationale: field("rationale")?.to_string(),
    };
    check_proposal_bounds(&change_set)?;
    Ok(change_set)
}

/// Strip one surrounding ```` ``` ```` fence (with an optional `json`
/// tag), if present — the single repair normalization, applied once.
fn strip_one_fence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    rest.strip_suffix("```").map(str::trim_end).unwrap_or(rest)
}

/// Re-check the change-set bounds the proposer contract names. Full
/// containment validation remains [`crate::changeset::validate_change_set`],
/// run by the loop; this is the cheap shape gate at the trust boundary.
fn check_proposal_bounds(change_set: &ChangeSet) -> Result<(), ProposeError> {
    let invalid = |message: &str| ProposeError::Invalid(message.to_string());
    if change_set.id.is_empty()
        || change_set.id.chars().count() > CHANGE_SET_ID_CHARS_MAX
        || !change_set
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(invalid(
            "planner id is empty, oversized, or outside [A-Za-z0-9._-]",
        ));
    }
    if change_set.payload.len() > CHANGE_SET_PAYLOAD_BYTES_MAX {
        return Err(invalid("planner payload exceeds the byte bound"));
    }
    if change_set.rationale.chars().count() > CHANGE_SET_RATIONALE_CHARS_MAX {
        return Err(invalid("planner rationale exceeds the char bound"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// A fake transport replaying queued results and recording the
    /// prompts it was handed — tests never touch a live daemon.
    #[derive(Debug)]
    struct FakeTransport {
        queue: RefCell<VecDeque<Result<String, String>>>,
        prompts: RefCell<Vec<String>>,
    }

    impl FakeTransport {
        fn new(results: Vec<Result<String, String>>) -> Self {
            FakeTransport {
                queue: RefCell::new(results.into()),
                prompts: RefCell::new(Vec::new()),
            }
        }
    }

    impl ChatTransport for FakeTransport {
        fn generate(&self, _model: &str, prompt: &str) -> Result<String, String> {
            self.prompts.borrow_mut().push(prompt.to_string());
            self.queue
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err("fake transport exhausted".to_string()))
        }
    }

    fn context() -> ProposeContext {
        ProposeContext {
            iteration: 3,
            incumbent_metric: Some(0.5),
            ledger_head_sha256: Some("ab".repeat(32)),
        }
    }

    fn fake_proposer(results: Vec<Result<String, String>>) -> OllamaProposer {
        OllamaProposer::new("planner-test", Box::new(FakeTransport::new(results)))
    }

    const VALID_JSON: &str = r#"{"id":"lr-up","kind":"trainlab_config","paths":[],
        "payload":"{\"learning_rate\":0.0002}","rationale":"raise the step size"}"#;

    #[test]
    fn parses_strict_json_change_set() {
        let mut proposer = fake_proposer(vec![Ok(VALID_JSON.to_string())]);
        let change_set = proposer.propose(&context()).expect("valid proposal");
        assert_eq!(change_set.id, "lr-up");
        assert_eq!(change_set.kind, ChangeKind::TrainlabConfig);
        assert_eq!(change_set.rationale, "raise the step size");
    }

    #[test]
    fn prompt_carries_run_state() {
        let prompt = build_prompt(&context());
        assert!(prompt.contains("Iteration: 3"), "{prompt}");
        assert!(prompt.contains("0.5000"), "{prompt}");
        assert!(prompt.contains("Ledger head: abab"), "{prompt}");
    }

    #[test]
    fn strips_one_code_fence() {
        let fenced = format!("```json\n{VALID_JSON}\n```");
        let mut proposer = fake_proposer(vec![Ok(fenced)]);
        let change_set = proposer
            .propose(&context())
            .expect("fenced proposal parses");
        assert_eq!(change_set.id, "lr-up");
    }

    #[test]
    fn daemon_unreachable_maps_to_invalid() {
        // A real HTTP transport against a closed loopback port: the
        // connection is refused immediately, no daemon involved.
        let transport = HttpTransport::new("http://127.0.0.1:9").expect("loopback parses");
        let mut proposer = OllamaProposer::new("planner-test", Box::new(transport));
        let err = proposer
            .propose(&context())
            .expect_err("unreachable daemon must fail");
        match err {
            ProposeError::Invalid(message) => {
                assert!(message.contains("planner transport"), "{message}");
            }
            ProposeError::Exhausted => panic!("wrong error class"),
        }
    }

    #[test]
    fn non_loopback_endpoint_is_refused_at_construction() {
        let err = HttpTransport::new("https://example.com:11434").expect_err("https refused");
        assert!(err.contains("http://"), "{err}");
        let err = HttpTransport::new("http://192.0.2.1:11434").expect_err("remote refused");
        assert!(err.contains("loopback"), "{err}");
        assert!(ProposerConfig::default().base_url == DEFAULT_BASE_URL);
    }

    #[test]
    fn free_form_and_malformed_output_are_invalid() {
        let mut proposer = fake_proposer(vec![Ok("I suggest raising the rate.".to_string())]);
        assert!(matches!(
            proposer.propose(&context()),
            Err(ProposeError::Invalid(_))
        ));
        let mut proposer = fake_proposer(vec![Ok(String::new())]);
        assert!(matches!(
            proposer.propose(&context()),
            Err(ProposeError::Invalid(_))
        ));
    }

    #[test]
    fn unknown_keys_and_kinds_are_invalid() {
        let extra = r#"{"id":"x","kind":"trainlab_config","paths":[],"payload":"{}",
            "rationale":"r","shell":"rm -rf /"}"#;
        let mut proposer = fake_proposer(vec![Ok(extra.to_string())]);
        assert!(matches!(
            proposer.propose(&context()),
            Err(ProposeError::Invalid(_))
        ));
        let bad_kind = r#"{"id":"x","kind":"shell","paths":[],"payload":"{}","rationale":"r"}"#;
        let mut proposer = fake_proposer(vec![Ok(bad_kind.to_string())]);
        assert!(matches!(
            proposer.propose(&context()),
            Err(ProposeError::Invalid(_))
        ));
    }

    #[test]
    fn oversized_payload_and_response_are_invalid() {
        let big_payload = format!(
            "{{\"id\":\"x\",\"kind\":\"trainlab_config\",\"paths\":[],\"payload\":\"{}\",\"rationale\":\"r\"}}",
            "a".repeat(CHANGE_SET_PAYLOAD_BYTES_MAX + 1)
        );
        let mut proposer = fake_proposer(vec![Ok(big_payload)]);
        assert!(matches!(
            proposer.propose(&context()),
            Err(ProposeError::Invalid(_))
        ));
        let huge_response = "a".repeat(PLANNER_RESPONSE_BYTES_MAX + 1);
        let mut proposer = fake_proposer(vec![Ok(huge_response)]);
        assert!(matches!(
            proposer.propose(&context()),
            Err(ProposeError::Invalid(_))
        ));
    }

    #[test]
    fn transport_error_detail_is_bounded() {
        let mut proposer = fake_proposer(vec![Err("x".repeat(10_000))]);
        match proposer.propose(&context()) {
            Err(ProposeError::Invalid(message)) => {
                assert!(
                    message.chars().count() <= ERROR_DETAIL_CHARS_MAX + 32,
                    "{}",
                    message.len()
                );
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    /// One generate envelope, as Ollama returns it.
    const ENVELOPE: &str = "{\"model\":\"m\",\"response\":\"hello\",\"done\":true}";

    #[test]
    fn content_length_response_parses() {
        let raw = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            ENVELOPE.len(),
            ENVELOPE
        );
        let response = parse_generate_response(raw.as_bytes()).expect("parses");
        assert_eq!(response, "hello");
    }

    #[test]
    fn chunked_response_parses() {
        // The envelope split across two chunks, an extension on the
        // second chunk size, and a trailer after the zero chunk —
        // the shape the live daemon actually sends for planner-sized
        // envelopes (the context array outgrows its write buffer).
        let split = ENVELOPE.len() / 2;
        let raw = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n\
             {:x}\r\n{}\r\n{:x};ext=1\r\n{}\r\n0\r\nx-trailer: y\r\n\r\n",
            split,
            &ENVELOPE[..split],
            ENVELOPE.len() - split,
            &ENVELOPE[split..]
        );
        let response = parse_generate_response(raw.as_bytes()).expect("parses");
        assert_eq!(response, "hello");
    }

    #[test]
    fn prompt_states_the_gate_rules() {
        // The planner is told the vocabulary and the shape rules it
        // will be held to: the delta fields, paths [] for config
        // change-sets, and the one worktree file a patch may touch.
        // (Found live: without them the planner guessed llama.cpp
        // knobs and put paths on a config change-set — every
        // proposal gate-rejected unmeasured.)
        let prompt = build_prompt(&context());
        assert!(prompt.contains("paths MUST be []"), "{prompt}");
        assert!(
            prompt.contains("temperature (number, 0.0 to 2.0)"),
            "{prompt}"
        );
        assert!(prompt.contains("invalid_penalty"), "{prompt}");
        assert!(prompt.contains("base-config.json"), "{prompt}");
    }

    #[test]
    fn chunked_framing_defects_fail_closed() {
        let head = "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n";
        let bad_size = format!("{head}zz\r\n{{}}\r\n0\r\n\r\n");
        assert!(parse_generate_response(bad_size.as_bytes()).is_err());
        let truncated = format!("{head}10\r\n{{}}\r\n");
        assert!(parse_generate_response(truncated.as_bytes()).is_err());
        let no_crlf = format!("{head}2\r\n{{}}0\r\n\r\n");
        assert!(parse_generate_response(no_crlf.as_bytes()).is_err());
    }
}
