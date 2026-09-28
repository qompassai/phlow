//! The line-delimited JSON control protocol, adapted from tuios's
//! `internal/session/verb_protocol.go`.
//!
//! One request per line:
//!
//! ```json
//! {"id": 1, "verb": "list-verbs", "params": {"session": "work"}}
//! ```
//!
//! and one response per line, either
//!
//! ```json
//! {"id": 1, "result": {"type": "window_list"}}
//! ```
//!
//! or
//!
//! ```json
//! {"id": 1, "error": {"code": "session_not_found", "message": "..."}}
//! ```
//!
//! Verified against the real daemon (probe 2026-09-28, Go 1.27.1):
//!
//! - the envelope id is opaque and echoed back verbatim: number, string,
//!   absent, and null ids all round-trip as themselves; a line whose JSON is
//!   malformed gets `invalid_request` with no id field at all;
//! - unknown verbs get `unknown_verb` with a hint naming `list-verbs`, the
//!   available verbs, and `did_you_mean` on a near-miss spelling;
//! - a missing `verb` field gets `invalid_request`; a wrongly-typed or
//!   unknown parameter gets `invalid_params` — unknown parameters are
//!   refused, never silently ignored;
//! - a request line past 16 MiB gets no response: the connection is dropped.
//!
//! Difference from tuios, documented once here: tuios sniffs the connection's
//! first byte to share the socket with a binary gob fast path. phlow has no
//! binary peer, so this socket is JSON-only and every line is a request.

use std::io::BufRead;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ErrorCode, TuiosError};

/// Bound: one request line, in bytes. Past it the server drops the
/// connection without answering, exactly like the probed daemon.
pub const FRAME_BYTES_MAX: usize = 16 * 1024 * 1024;
/// Bound: verb name length, in bytes.
pub const VERB_BYTES_MAX: usize = 128;
/// Bound: characters examined for a `did_you_mean` suggestion, in bytes.
pub const VERB_SUGGEST_BYTES_MAX: usize = 128;

/// Deserialize an optional id while keeping an explicit JSON `null` distinct
/// from an absent field. Plain `Option<Value>` collapses present-null into
/// `None`, but the probed daemon echoes explicit null as `"id":null` and
/// omits the field only when it was absent.
fn de_opt_id<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

/// One decoded request line. The id is opaque JSON — number, string, object,
/// array, null, or absent — and is echoed back on the response verbatim.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Opaque id, echoed verbatim. `None` when the field was absent;
    /// `Some(Value::Null)` when it was explicitly null.
    #[serde(default, deserialize_with = "de_opt_id")]
    pub id: Option<Value>,
    /// The verb to dispatch. Required and non-empty.
    pub verb: String,
    /// Raw params for the verb's handler; may be absent.
    pub params: Option<Value>,
}

/// Hint attached to an error envelope. Additive and omitempty: a consumer
/// that reads only code and message is unaffected. Mirrors tuios's `VerbHint`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VerbHint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub did_you_mean: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub available: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accepted: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The error half of a response line.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<VerbHint>,
}

/// One response line. Exactly one of `result` / `error` is set; the id echoes
/// the request's, including its absence.
#[derive(Debug, Clone, Serialize)]
pub struct Response {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorEnvelope>,
}

impl Response {
    /// A success response echoing the request's id.
    #[must_use]
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: Option::None,
        }
    }

    /// A failure response echoing the request's id (or none, when the request
    /// line never decoded far enough to have one).
    #[must_use]
    pub fn err(
        id: Option<Value>,
        code: ErrorCode,
        message: String,
        hint: Option<VerbHint>,
    ) -> Self {
        Self {
            id,
            result: Option::None,
            error: Some(ErrorEnvelope {
                code: code.as_str().to_owned(),
                message,
                hint,
            }),
        }
    }

    /// Serialize as one newline-terminated JSON line.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = serde_json::to_vec(self).expect("response is JSON-serializable");
        out.push(b'\n');
        out
    }
}

/// A verb-level failure: always becomes one error response line, never drops
/// the connection. Transport failures ([`TuiosError::Io`],
/// [`TuiosError::FrameTooLarge`]) are the ones that end a connection.
#[derive(Debug, Clone)]
pub struct ProtocolError {
    /// Stable wire code.
    pub code: ErrorCode,
    /// Bounded human text; never echoes unbounded peer input.
    pub message: String,
    /// Additive hint, boxed: hints are cold data, and boxing keeps the
    /// error small enough to return by value without tripping
    /// `result_large_err`. A client reading only code and message is
    /// unaffected.
    pub hint: Option<Box<VerbHint>>,
}

/// Bound: error message text a failure can carry, in bytes.
pub const ERROR_MESSAGE_BYTES_MAX: usize = 1024;

impl ProtocolError {
    /// Build with a hint. The message is truncated to
    /// [`ERROR_MESSAGE_BYTES_MAX`] bytes.
    pub fn new(code: ErrorCode, message: impl Into<String>, hint: VerbHint) -> Self {
        Self::with_optional_hint(code, message, Some(hint))
    }

    /// Build without a hint.
    pub fn without_hint(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::with_optional_hint(code, message, Option::None)
    }

    fn with_optional_hint(
        code: ErrorCode,
        message: impl Into<String>,
        hint: Option<VerbHint>,
    ) -> Self {
        let message: String = message.into();
        let message = crate::truncate_bytes(&message, ERROR_MESSAGE_BYTES_MAX);
        Self {
            code,
            message,
            hint: hint.map(Box::new),
        }
    }

    /// Render as the response line for the request carrying `id`.
    #[must_use]
    pub fn into_response(self, id: Option<Value>) -> Response {
        Response::err(id, self.code, self.message, self.hint.map(|hint| *hint))
    }
}

impl From<TuiosError> for ProtocolError {
    /// Transport and domain errors become `internal`, except protocol errors
    /// which keep their code. A frame-too-large never reaches here: the
    /// server drops the connection instead of answering.
    fn from(err: TuiosError) -> Self {
        match err {
            TuiosError::Protocol { code, message } => Self::without_hint(code, message),
            TuiosError::Io(io) => Self::without_hint(ErrorCode::Internal, format!("io: {io}")),
            TuiosError::FrameTooLarge { .. } => {
                Self::without_hint(ErrorCode::Internal, "frame too large")
            }
        }
    }
}

/// Read one request frame: bytes up to the next `\n`, with the trailing
/// newline (and a preceding `\r`) stripped. Returns `None` on clean EOF with
/// no pending bytes. Allocation never exceeds [`FRAME_BYTES_MAX`]: the line
/// is consumed in buffer-sized chunks and the length is checked before each
/// extend, so a hostile peer cannot make the reader allocate past the cap.
///
/// # Errors
///
/// `FrameTooLarge` when the line passes [`FRAME_BYTES_MAX`]; `Io` on read
/// failure. Both end the connection without a response, matching the probed
/// daemon's drop behavior.
pub fn read_frame<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>, TuiosError> {
    let mut frame: Vec<u8> = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(if frame.is_empty() {
                Option::None
            } else {
                Some(frame)
            });
        }
        let newline = chunk.iter().position(|&b| b == b'\n');
        let take = newline.map_or(chunk.len(), |i| i + 1);
        let new_len = frame
            .len()
            .checked_add(take)
            .filter(|&n| n <= FRAME_BYTES_MAX)
            .ok_or(TuiosError::FrameTooLarge {
                bytes: FRAME_BYTES_MAX as u64 + 1,
            })?;
        debug_assert!(new_len <= FRAME_BYTES_MAX);
        frame.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if newline.is_some() {
            break;
        }
    }
    if frame.last() == Some(&b'\n') {
        frame.pop();
    }
    if frame.last() == Some(&b'\r') {
        frame.pop();
    }
    Ok(Some(frame))
}

/// Decode one request line into the envelope, validating its shape. Unknown
/// verbs are *not* rejected here — the dispatcher owns the registry and its
/// `did_you_mean` hint.
///
/// # Errors
///
/// `invalid_request` when the line is not a JSON object, or the `verb` field
/// is missing, empty, over-long, or not a string. The id is never available
/// on a malformed line, so the error carries none — as probed.
pub fn decode_request(line: &[u8]) -> Result<Request, ProtocolError> {
    #[derive(Deserialize)]
    struct Wire {
        #[serde(default, deserialize_with = "de_opt_id")]
        id: Option<Value>,
        verb: Option<Value>,
        params: Option<Value>,
    }
    let wire: Wire = serde_json::from_slice(line).map_err(|err| {
        ProtocolError::without_hint(
            ErrorCode::InvalidRequest,
            format!("malformed JSON request: {err}"),
        )
    })?;
    let verb = match wire.verb {
        Some(Value::String(s)) if !s.is_empty() && s.len() <= VERB_BYTES_MAX => s,
        _ => {
            let hint = VerbHint {
                param: Some("verb".to_owned()),
                detail: Some(
                    "Every request line is an object of the form \
                     {\"id\":1,\"verb\":\"list-verbs\",\"params\":{}}."
                        .to_owned(),
                ),
                ..Default::default()
            };
            return Err(ProtocolError::new(
                ErrorCode::InvalidRequest,
                "request is missing the \"verb\" field",
                hint,
            ));
        }
    };
    Ok(Request {
        id: wire.id,
        verb,
        params: wire.params,
    })
}

/// Suggest the closest registered verb for a near-miss spelling, or `None`.
/// Bounded: candidates and input past [`VERB_SUGGEST_BYTES_MAX`] bytes are
/// skipped, and only a suggestion within a small edit distance is returned.
#[must_use]
pub fn closest_verb<'a>(name: &str, verbs: &'a [String]) -> Option<&'a str> {
    const DISTANCE_MAX: usize = 3;
    if name.len() > VERB_SUGGEST_BYTES_MAX {
        return Option::None;
    }
    let mut best: Option<(&str, usize)> = Option::None;
    for verb in verbs {
        if verb.len() > VERB_SUGGEST_BYTES_MAX {
            continue;
        }
        let distance = levenshtein(name, verb, DISTANCE_MAX);
        if distance <= DISTANCE_MAX && best.is_none_or(|(_, d)| distance < d) {
            best = Some((verb, distance));
        }
    }
    best.map(|(verb, _)| verb)
}

/// Levenshtein distance with an early exit past `cap`. Verb names are short;
/// the full matrix is bounded by 128x128 cells.
fn levenshtein(a: &str, b: &str, cap: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len() > VERB_SUGGEST_BYTES_MAX || b.len() > VERB_SUGGEST_BYTES_MAX {
        return cap + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        let mut row_min = curr[0];
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j] + cost).min((curr[j] + 1).min(prev[j + 1] + 1));
            row_min = row_min.min(curr[j + 1]);
        }
        if row_min > cap {
            return cap + 1;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // --- validation ---

    #[test]
    fn request_round_trip_with_numeric_id() {
        let line = br#"{"id": 1, "verb": "ping", "params": {"a": true}}"#;
        let req = decode_request(line).expect("decode");
        assert_eq!(req.id, Some(Value::from(1)));
        assert_eq!(req.verb, "ping");
        let resp = Response::ok(req.id.clone(), Value::from(true));
        let encoded = String::from_utf8(resp.encode()).expect("utf8");
        let back: Value = serde_json::from_str(&encoded).expect("json");
        assert_eq!(back["id"], Value::from(1));
        assert_eq!(back["result"], Value::from(true));
        assert!(back.get("error").is_none());
    }

    #[test]
    fn string_id_echoes_verbatim() {
        let req = decode_request(br#"{"id": "abc-123", "verb": "ping"}"#).expect("decode");
        assert_eq!(req.id, Some(Value::from("abc-123")));
        let encoded = String::from_utf8(Response::ok(req.id, Value::Null).encode()).expect("utf8");
        assert!(encoded.contains(r#""id":"abc-123""#), "{encoded}");
    }

    #[test]
    fn absent_id_stays_absent() {
        let req = decode_request(br#"{"verb": "ping"}"#).expect("decode");
        assert_eq!(req.id, Option::None);
        let encoded = String::from_utf8(Response::ok(req.id, Value::Null).encode()).expect("utf8");
        assert!(!encoded.contains("\"id\""), "{encoded}");
    }

    #[test]
    fn null_id_echoes_as_null() {
        let req = decode_request(br#"{"id": null, "verb": "ping"}"#).expect("decode");
        assert_eq!(req.id, Some(Value::Null));
        let encoded = String::from_utf8(Response::ok(req.id, Value::Null).encode()).expect("utf8");
        assert!(encoded.contains(r#""id":null"#), "{encoded}");
    }

    #[test]
    fn object_id_is_opaque_and_echoes() {
        let req = decode_request(br#"{"id": {"n": 1}, "verb": "ping"}"#).expect("decode");
        let encoded = String::from_utf8(Response::ok(req.id, Value::Null).encode()).expect("utf8");
        assert!(encoded.contains(r#""id":{"n":1}"#), "{encoded}");
    }

    #[test]
    fn read_frame_strips_crlf() {
        let mut cursor = Cursor::new(b"{\"a\":1}\r\n".to_vec());
        let frame = read_frame(&mut cursor).expect("read").expect("frame");
        assert_eq!(frame, b"{\"a\":1}");
    }

    #[test]
    fn closest_verb_suggests_near_miss() {
        let verbs = ["list-verbs".to_owned(), "new-session".to_owned()];
        assert_eq!(closest_verb("list-verbz", &verbs), Some("list-verbs"));
        assert_eq!(closest_verb("frobnicate", &verbs), Option::None);
    }

    #[test]
    fn response_encode_is_one_newline_terminated_line() {
        let encoded = Response::ok(Some(Value::from(1)), json_obj()).encode();
        assert!(encoded.ends_with(b"\n"));
        assert_eq!(encoded.iter().filter(|&&b| b == b'\n').count(), 1);
    }

    fn json_obj() -> Value {
        serde_json::json!({"a": [1, 2], "b": "x"})
    }

    #[test]
    fn verb_hint_omits_empty_fields() {
        let hint = VerbHint::default();
        let encoded = serde_json::to_string(&hint).expect("json");
        assert_eq!(encoded, "{}");
    }

    // --- adversarial ---

    #[test]
    fn oversize_frame_is_refused_without_allocating_past_cap() {
        let huge = vec![b'x'; FRAME_BYTES_MAX + 1];
        let mut cursor = Cursor::new(huge);
        let err = read_frame(&mut cursor).expect_err("oversize must fail");
        assert!(matches!(err, TuiosError::FrameTooLarge { .. }));
    }

    #[test]
    fn malformed_json_carries_no_id() {
        let err = decode_request(b"{\"id\":4,\"verb\":\"ping\"").expect_err("truncated");
        assert_eq!(err.code, ErrorCode::InvalidRequest);
        let resp = err.into_response(Option::None);
        let encoded = String::from_utf8(resp.encode()).expect("utf8");
        assert!(!encoded.contains("\"id\""), "{encoded}");
    }

    #[test]
    fn non_utf8_line_is_invalid_request() {
        let err = decode_request(&[0xff, 0xfe, b'{', b'}']).expect_err("non-utf8");
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn missing_verb_is_invalid_request_with_hint() {
        let err = decode_request(br#"{"id": 5}"#).expect_err("missing verb");
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn non_string_verb_is_invalid_request() {
        let err = decode_request(br#"{"id": 5, "verb": 42}"#).expect_err("numeric verb");
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn empty_verb_is_invalid_request() {
        let err = decode_request(br#"{"verb": ""}"#).expect_err("empty verb");
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn overlong_verb_is_invalid_request() {
        let line = format!("{{\"verb\": \"{}\"}}", "v".repeat(VERB_BYTES_MAX + 1));
        let err = decode_request(line.as_bytes()).expect_err("overlong verb");
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn error_response_carries_code_and_hint() {
        let hint = VerbHint {
            did_you_mean: Some("list-verbs".to_owned()),
            ..Default::default()
        };
        let resp = Response::err(
            Some(Value::from(3)),
            ErrorCode::UnknownVerb,
            "unknown verb list-verbz".to_owned(),
            Some(hint),
        );
        let encoded = String::from_utf8(resp.encode()).expect("utf8");
        let back: Value = serde_json::from_str(&encoded).expect("json");
        assert_eq!(back["error"]["code"], Value::from("unknown_verb"));
        assert_eq!(
            back["error"]["hint"]["did_you_mean"],
            Value::from("list-verbs")
        );
        assert!(back.get("result").is_none());
    }
}
