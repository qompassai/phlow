//! Completion samplers: the contract, a scripted sampler for tests,
//! and a loopback Ollama sampler for real specialists.
//!
//! A sampler only *produces* completions; scoring is the executor's
//! job. Samplers must return exactly the requested number of
//! completions or fail — a short group is an error, never a smaller
//! group (a smaller group would silently change the statistics).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde::Deserialize;

use crate::error::TrainlabError;

/// Maximum completions requested in one `sample` call.
pub const GROUP_SIZE_MAX: usize = 64;
/// Default Ollama base URL (loopback, per phlow's local-first rule).
pub const OLLAMA_BASE_URL_DEFAULT: &str = "http://127.0.0.1:11434";
/// Bound on an Ollama response body (bytes).
pub const RESPONSE_BYTES_MAX: usize = 1024 * 1024;
/// Default per-request deadline for Ollama calls.
pub const REQUEST_DEADLINE_DEFAULT: Duration = Duration::from_secs(120);

/// A source of completions for one prompt.
pub trait Sampler {
    /// Stable identifier recorded in receipts (model name, stub id).
    fn id(&self) -> String;

    /// Sample exactly `group_size` completions for `prompt`.
    ///
    /// `temperature` and `seed` are part of the sampling contract;
    /// deterministic samplers must produce identical output for
    /// identical `(prompt, group_size, temperature, seed)`.
    fn sample(
        &self,
        prompt: &str,
        group_size: usize,
        temperature: f64,
        seed: u64,
    ) -> Result<Vec<String>, TrainlabError>;
}

/// Validate shared sampling parameters.
fn validate_sample_args(group_size: usize, temperature: f64) -> Result<(), TrainlabError> {
    if group_size == 0 || group_size > GROUP_SIZE_MAX {
        return Err(TrainlabError::LimitExceeded(format!(
            "group_size {group_size} outside 1..={GROUP_SIZE_MAX}"
        )));
    }
    if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
        return Err(TrainlabError::InvalidConfig(format!(
            "temperature {temperature} outside 0.0..=2.0"
        )));
    }
    Ok(())
}

/// A deterministic sampler that cycles a fixed list of completions.
///
/// For tests and plumbing smoke runs: canned completions whose
/// rewards are known in advance, so end-to-end statistics can be
/// asserted exactly.
#[derive(Debug, Clone)]
pub struct ScriptedSampler {
    id: String,
    responses: Vec<String>,
}

impl ScriptedSampler {
    /// Build from a non-empty response list, cycled in order.
    pub fn new(
        id: impl Into<String>,
        responses: Vec<String>,
    ) -> Result<ScriptedSampler, TrainlabError> {
        if responses.is_empty() {
            return Err(TrainlabError::InvalidConfig(
                "scripted sampler needs at least one response".to_string(),
            ));
        }
        Ok(ScriptedSampler {
            id: id.into(),
            responses,
        })
    }
}

impl Sampler for ScriptedSampler {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn sample(
        &self,
        _prompt: &str,
        group_size: usize,
        temperature: f64,
        _seed: u64,
    ) -> Result<Vec<String>, TrainlabError> {
        validate_sample_args(group_size, temperature)?;
        Ok((0..group_size)
            .map(|index| self.responses[index % self.responses.len()].clone())
            .collect())
    }
}

/// Sampler backed by a local Ollama server's `/api/generate`.
///
/// Loopback by default; a non-loopback base URL is refused unless
/// `allow_remote` is set — phlow's standing local-first rule.
#[derive(Debug, Clone)]
pub struct OllamaSampler {
    base_url: String,
    model: String,
    deadline: Duration,
}

/// Ollama `/api/generate` response (only the fields we consume).
#[derive(Debug, Deserialize)]
struct OllamaResponse {
    response: String,
}

impl OllamaSampler {
    /// Create a sampler for `model` on `base_url`.
    ///
    /// Fails when the URL is malformed, or non-loopback without
    /// `allow_remote`.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        allow_remote: bool,
    ) -> Result<OllamaSampler, TrainlabError> {
        let base_url = base_url.into();
        let host = url_host(&base_url).ok_or_else(|| {
            TrainlabError::InvalidConfig(format!("malformed base URL: {base_url}"))
        })?;
        let loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
        if !loopback && !allow_remote {
            return Err(TrainlabError::InvalidConfig(format!(
                "non-loopback Ollama URL {base_url} requires explicit remote opt-in"
            )));
        }
        Ok(OllamaSampler {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.into(),
            deadline: REQUEST_DEADLINE_DEFAULT,
        })
    }

    /// One `/api/generate` call; one completion.
    ///
    /// The response is normalized by [`extract_completion`]: phlow's
    /// specialists are instruct models that answer with prose and
    /// fenced code, while the reward harness needs the raw body a
    /// base model would have continued with. This is the same
    /// pattern phlow already uses for the reviewer's verdicts — one
    /// strict normalization step ahead of a strict consumer — and it
    /// is deliberately conservative: it can only *select* lines from
    /// the response, never invent code.
    fn generate_once(
        &self,
        prompt: &str,
        temperature: f64,
        seed: u64,
    ) -> Result<String, TrainlabError> {
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "options": {"temperature": temperature, "seed": seed},
        })
        .to_string();
        let response = http_post_json(&self.base_url, "/api/generate", &body, self.deadline)?;
        let parsed: OllamaResponse = serde_json::from_str(&response)
            .map_err(|err| TrainlabError::Sampler(format!("bad Ollama response: {err}")))?;
        Ok(extract_completion(&parsed.response))
    }
}

impl Sampler for OllamaSampler {
    fn id(&self) -> String {
        format!("ollama:{}", self.model)
    }

    fn sample(
        &self,
        prompt: &str,
        group_size: usize,
        temperature: f64,
        seed: u64,
    ) -> Result<Vec<String>, TrainlabError> {
        validate_sample_args(group_size, temperature)?;
        let mut completions = Vec::with_capacity(group_size);
        for index in 0..group_size {
            let call_seed = seed.wrapping_add(index as u64);
            completions.push(self.generate_once(prompt, temperature, call_seed)?);
        }
        Ok(completions)
    }
}

/// Extract the function body from a model response.
///
/// Rules, in order: take the first fenced code block if one exists
/// (else the whole response); drop any `def ...` line (the prompt
/// already carries it) and everything before it; then keep blank and
/// indented lines, stopping at the first non-blank column-0 line
/// once the body has started. Returns an empty string when no body
/// is present — the executor classifies that as `Invalid`, which is
/// the honest score for a non-answer.
pub fn extract_completion(response: &str) -> String {
    let code = first_fenced_block(response).unwrap_or(response);
    let mut body: Vec<&str> = Vec::new();
    let mut started = false;
    for line in code.lines() {
        if line.starts_with("def ") {
            body.clear();
            started = false;
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented {
            started = true;
            body.push(line);
        } else if line.trim().is_empty() {
            if started {
                body.push(line);
            }
        } else if started {
            break;
        }
    }
    if body.is_empty() {
        return String::new();
    }
    // The prompt ends at the `def` line's colon; the body must start
    // on its own line or multi-statement bodies would not parse.
    let mut out = String::from("\n");
    out.push_str(&body.join("\n"));
    out.push('\n');
    out
}

/// The content of the first ``` fenced block, if any.
fn first_fenced_block(text: &str) -> Option<&str> {
    let open = text.find("```")?;
    let after_open = &text[open + 3..];
    let content_start = after_open.find('\n')? + 1;
    let content = &after_open[content_start..];
    let close = content.find("```")?;
    Some(&content[..close])
}

/// Extract the host from an `http(s)://host[:port]/...` URL without a
/// URL crate; returns `None` for anything malformed.
fn url_host(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let host_port = rest.split('/').next()?;
    let host = host_port.rsplit('@').next()?;
    let host = host.strip_prefix('[').map_or(host, |stripped| {
        stripped.split(']').next().unwrap_or(stripped)
    });
    let host = host.split(':').next()?;
    if host.is_empty() { None } else { Some(host) }
}

/// Minimal bounded HTTP/1.1 POST returning the response body.
///
/// Hand-rolled over `TcpStream` so the crate needs no HTTP client
/// dependency for one loopback endpoint. The body read is capped at
/// [`RESPONSE_BYTES_MAX`]; exceeding it is an error, and only HTTP
/// 200 is success. Bodies framed with `Transfer-Encoding: chunked`
/// are de-chunked: Go's server (Ollama) frames any response over its
/// ~2 KB write buffer that way, which is every real completion, so
/// a reader that skips de-chunking hands the chunk-size line to the
/// JSON parser and fails on exactly the responses that matter.
fn http_post_json(
    base_url: &str,
    path: &str,
    body: &str,
    deadline: Duration,
) -> Result<String, TrainlabError> {
    let host = url_host(base_url)
        .ok_or_else(|| TrainlabError::Sampler(format!("malformed base URL: {base_url}")))?;
    let port = url_port(base_url);
    let mut stream = TcpStream::connect((host, port))
        .map_err(|err| TrainlabError::Sampler(format!("connect {host}:{port}: {err}")))?;
    stream.set_read_timeout(Some(deadline)).ok();
    stream.set_write_timeout(Some(deadline)).ok();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| TrainlabError::Sampler(format!("write request: {err}")))?;
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                if raw.len() + read > RESPONSE_BYTES_MAX + 4096 {
                    return Err(TrainlabError::LimitExceeded(
                        "Ollama response exceeds size bound".to_string(),
                    ));
                }
                raw.extend_from_slice(&chunk[..read]);
            }
            Err(err) => return Err(TrainlabError::Sampler(format!("read response: {err}"))),
        }
    }
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| TrainlabError::Sampler("malformed HTTP response".to_string()))?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        let status = head.lines().next().unwrap_or("unknown status");
        return Err(TrainlabError::Sampler(format!("Ollama returned {status}")));
    }
    let payload = &raw[split + 4..];
    let body_bytes = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        decode_chunked(payload)?
    } else {
        if payload.len() > RESPONSE_BYTES_MAX {
            return Err(TrainlabError::LimitExceeded(
                "Ollama response exceeds size bound".to_string(),
            ));
        }
        payload.to_vec()
    };
    Ok(String::from_utf8_lossy(&body_bytes).into_owned())
}

/// Decode an HTTP/1.1 chunked body. Each chunk is a hex size line
/// (extensions after `;` are ignored), exactly that many bytes, and
/// a CRLF; a zero-size chunk ends the body. Malformed sizes,
/// truncated chunks, and missing CRLFs are errors — a partially
/// decoded body is never returned.
fn decode_chunked(mut rest: &[u8]) -> Result<Vec<u8>, TrainlabError> {
    let malformed =
        |detail: &str| TrainlabError::Sampler(format!("malformed chunked body: {detail}"));
    let mut out: Vec<u8> = Vec::new();
    loop {
        let line_end = rest
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| malformed("chunk size line has no CRLF"))?;
        let size_text = String::from_utf8_lossy(&rest[..line_end]).into_owned();
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| malformed("chunk size is not hex"))?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        let end = size
            .checked_add(2)
            .filter(|end| *end <= rest.len())
            .ok_or_else(|| malformed("chunk is truncated"))?;
        if &rest[size..end] != b"\r\n" {
            return Err(malformed("chunk is not followed by CRLF"));
        }
        out.extend_from_slice(&rest[..size]);
        if out.len() > RESPONSE_BYTES_MAX {
            return Err(TrainlabError::LimitExceeded(
                "Ollama response exceeds size bound".to_string(),
            ));
        }
        rest = &rest[end..];
    }
}

/// Port from a URL, defaulting to Ollama's 11434.
fn url_port(url: &str) -> u16 {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let host_port = rest.split('/').next().unwrap_or("");
    host_port
        .rsplit(':')
        .next()
        .and_then(|port| port.parse::<u16>().ok())
        .unwrap_or(11434)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_sampler_cycles_and_counts_exactly() {
        let sampler = ScriptedSampler::new("stub", vec!["a".into(), "b".into()]).expect("sampler");
        let out = sampler.sample("prompt", 5, 0.35, 1).expect("sample");
        assert_eq!(out, vec!["a", "b", "a", "b", "a"]);
    }

    #[test]
    fn sampler_args_are_bounded() {
        let sampler = ScriptedSampler::new("stub", vec!["a".into()]).expect("sampler");
        assert!(sampler.sample("p", 0, 0.3, 1).is_err());
        assert!(sampler.sample("p", GROUP_SIZE_MAX + 1, 0.3, 1).is_err());
        assert!(sampler.sample("p", 2, 2.5, 1).is_err());
        assert!(sampler.sample("p", 2, f64::NAN, 1).is_err());
    }

    #[test]
    fn ollama_remote_requires_opt_in() {
        // Adversarial: a remote endpoint must never be reachable by
        // a default configuration.
        assert!(OllamaSampler::new("http://example.com:11434", "m", false).is_err());
        assert!(OllamaSampler::new("http://example.com:11434", "m", true).is_ok());
        assert!(OllamaSampler::new("http://127.0.0.1:11434", "m", false).is_ok());
        assert!(OllamaSampler::new("not a url", "m", false).is_err());
    }

    #[test]
    fn url_host_parsing() {
        assert_eq!(url_host("http://127.0.0.1:11434"), Some("127.0.0.1"));
        assert_eq!(url_host("http://localhost:11434/"), Some("localhost"));
        assert_eq!(url_host("ftp://example.com"), None);
        assert_eq!(url_host("http://"), None);
    }

    #[test]
    fn extraction_from_chatty_fenced_response() {
        // The shape a real instruct model returned for a task prompt:
        // prose, a fenced full function, then an example block. Only
        // the body may survive.
        let response = "Certainly! Below is the completed function:\n\n```python\ndef increment_abc(x):\n    return x + 1\n```\n\nExample:\n\n```python\nresult = increment_abc(5)\n```";
        assert_eq!(extract_completion(response), "\n    return x + 1\n");
    }

    #[test]
    fn extraction_from_raw_continuation() {
        assert_eq!(
            extract_completion("\n    return x + 1\n"),
            "\n    return x + 1\n"
        );
        // Multi-statement bodies stay intact and parseable.
        let multi = "\n    total = x + 1\n    return total\nprint('junk')\n";
        assert_eq!(
            extract_completion(multi),
            "\n    total = x + 1\n    return total\n"
        );
    }

    #[test]
    fn chunked_body_decodes_across_chunk_boundaries() {
        // Two chunks splitting the JSON mid-token, plus an extension
        // on the second size line: the reassembled body must be the
        // exact payload bytes.
        let framed = b"7\r\n{\"respo\r\na;ext=1\r\nnse\": \"x\"}\r\n0\r\n\r\n";
        let decoded = decode_chunked(framed).expect("decode");
        assert_eq!(decoded, b"{\"response\": \"x\"}");
        // An unframed (Content-Length) empty body is not chunked,
        // but an immediate zero chunk is a valid empty body.
        assert_eq!(
            decode_chunked(b"0\r\n\r\n").expect("empty"),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn chunked_body_rejects_malformed_framing() {
        // Adversarial: truncated, mis-sized, or unterminated framing
        // must error, never yield a partial body to the JSON parser.
        assert!(decode_chunked(b"5\r\nabc").is_err(), "truncated chunk");
        assert!(
            decode_chunked(b"zz\r\nabcde\r\n0\r\n\r\n").is_err(),
            "non-hex size"
        );
        assert!(
            decode_chunked(b"3\r\nabcXX0\r\n\r\n").is_err(),
            "missing chunk CRLF"
        );
        assert!(
            decode_chunked(b"3\r\nabc\r\n").is_err(),
            "no terminating zero chunk"
        );
    }

    #[test]
    fn http_post_json_reads_a_chunked_response() {
        // End-to-end against a local listener serving the framing
        // Ollama's Go server uses for responses over ~2 KB: the
        // helper must return the de-chunked JSON body.
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut byte = [0_u8; 1];
            // Read the whole request (headers + Content-Length
            // body): closing a socket with unread request bytes
            // resets the connection and can eat the response.
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout");
            loop {
                if stream.read(&mut byte).expect("read") == 0 {
                    break;
                }
                request.push(byte[0]);
                if let Some(split) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..split]).into_owned();
                    let body_len: usize = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= split + 4 + body_len {
                        break;
                    }
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                      7\r\n{\"respo\r\na\r\nnse\": \"x\"}\r\n0\r\n\r\n",
                )
                .expect("write response");
        });
        let body = http_post_json(
            &format!("http://127.0.0.1:{port}"),
            "/api/generate",
            "{}",
            Duration::from_secs(5),
        )
        .expect("chunked response");
        assert_eq!(body, "{\"response\": \"x\"}");
        server.join().expect("server thread");
    }

    #[test]
    fn extraction_of_non_answer_is_empty() {
        // Adversarial: a response with no code body extracts to
        // empty, which the executor scores Invalid — never a pass.
        assert_eq!(extract_completion("I cannot help with that."), "");
        assert_eq!(extract_completion(""), "");
    }
}
