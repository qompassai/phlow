//! Era guard: the byte-level front door of the MCP server.
//!
//! Phlow's framing was hand-rolled (dual-era) until 0.2; the domain
//! logic now runs behind the rmcp SDK (the fleet standard, D-113),
//! but the SDK does not by itself give the fleet its modern-only
//! *posture*. Left to itself the SDK negotiates any era a client
//! asks for, and a refused handshake tears the connection down.
//! This guard sits in front of the SDK on stdio and owns era
//! admission, following the fleet's guard pattern (WS2, proven on
//! theodolite/lumen/bloch):
//!
//! - `initialize` (any version) → the designed modern-only
//!   refusal, `-32022` naming the supported versions; the
//!   connection and the process stay up, so a probe can continue
//!   with modern frames
//! - malformed JSON → `-32700` (`parse error: {serde detail}`,
//!   id null); a line reaching the line bound → `-32700` naming
//!   the cap, remainder drained; the server stays alive
//!   throughout. (The hand-rolled server answered oversize with a
//!   terminal `-32700` and closed the connection; the fleet's
//!   drain-and-stay-alive semantics are a deliberate, itemized
//!   delta — FLOWGUARD-MAPPING A3, accepted by Pax.)
//! - a non-object frame → `-32600` (the fleet framing contract)
//! - notifications (no `id`) are dropped silently and never
//!   executed — the fleet contract for every migrated server,
//!   held here because this surface carries write-class tools
//!   (`flow_run`)
//! - every other frame forwards to the SDK **verbatim**: method
//!   dispatch, typed-envelope validation, and the per-request
//!   `_meta` version gate are the SDK's. The handler additionally
//!   pins `supported_protocol_versions` to 2026-07-28, so
//!   `server/discover` advertises exactly one version.
//!
//! Two checks live here because the raw frame is the only place
//! they are visible — the SDK's typed dispatch drops both before
//! a handler could see them (FLOWGUARD-MAPPING GAP-1/GAP-2):
//!
//! - **Request-id validity.** The hand-rolled server accepted any
//!   integer id (Python's arbitrary-precision `int`) and rejected
//!   null/bool/float/array/object ids with `-32600`. The SDK's
//!   `RequestId` is `i64`-bounded, so an out-of-range or malformed
//!   id is refused here with the server's own `-32600` shape
//!   (null id) instead of dying inside SDK deserialization.
//!   Accepting and echoing a >i64 id is the one flowguard
//!   behavior with no rmcp equivalent (GAP-1 fallback (ii),
//!   ruled by Pax): refused, never silently mangled.
//! - **Strict `tools/call` params.** Unknown params keys →
//!   `-32602` "Unknown tools/call parameters", and a non-object
//!   `params` → `-32602` "Params must be an object" — the exact
//!   refusals the hand-rolled dispatch produced (GAP-2).
//!
//! Guard-written frames never overtake an unanswered forwarded
//! request: the [`Ordering`] tracker reproduces the sequential
//! answer order the hand-rolled stdio loop had.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, Stdin, Stdout,
    WriteHalf,
};
use tokio::sync::Mutex;

use crate::json_ascii::is_integer_text;
use crate::protocol::{INVALID_PARAMS, INVALID_REQUEST, MAX_FRAME_BYTES, PARSE_ERROR};

/// The one protocol version this server speaks.
pub const MODERN_VERSION: &str = "2026-07-28";
/// Maximum bytes in one JSON-RPC line: phlow's own frame bound
/// ([`MAX_FRAME_BYTES`], 1 MiB), carried over from the hand-rolled
/// transport unchanged.
pub const MAX_LINE_BYTES: usize = MAX_FRAME_BYTES;

const UNSUPPORTED_PROTOCOL_VERSION: i32 = -32022;

/// Stdout shared by the guard (its own error frames) and the pump
/// (SDK responses); one line is written per lock, never interleaved.
pub type SharedStdout = Arc<Mutex<Stdout>>;

/// Response-ordering state shared by the guard and the pump: the
/// ids of forwarded requests whose SDK response has not yet been
/// written to stdout. The hand-rolled server answered strictly in
/// request order; the guard therefore never writes one of its own
/// frames while an earlier forwarded request is unanswered. This
/// is the WS2 Wave 1 ordering fix, carried in from the start.
pub struct Ordering {
    outstanding: Mutex<Vec<Value>>,
    drained: tokio::sync::Notify,
}

impl Ordering {
    pub fn new() -> Arc<Self> {
        Arc::new(Ordering {
            outstanding: Mutex::new(Vec::new()),
            drained: tokio::sync::Notify::new(),
        })
    }

    async fn track(&self, id: Value) {
        self.outstanding.lock().await.push(id);
    }

    async fn complete(&self, id: &Value) {
        {
            let mut queue = self.outstanding.lock().await;
            if let Some(pos) = queue.iter().position(|q| q == id) {
                queue.remove(pos);
            }
        }
        self.drained.notify_waiters();
    }

    /// Wait until no forwarded request is unanswered. Bounded: if
    /// the SDK died mid-request its response will never come, and
    /// the guard must go on answering framing errors regardless.
    async fn wait_drained(&self) {
        let deadline = std::time::Duration::from_secs(5);
        let _ = tokio::time::timeout(deadline, async {
            loop {
                let notified = self.drained.notified();
                if self.outstanding.lock().await.is_empty() {
                    return;
                }
                notified.await;
            }
        })
        .await;
    }
}

fn error_frame(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The designed refusal of legacy-era contact (CHECKLIST 1.8 shape:
/// the SDK's own `unsupported_protocol_version` payload).
fn unsupported_version_frame(id: Value, requested: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": UNSUPPORTED_PROTOCOL_VERSION,
            "message": "Unsupported protocol version",
            "data": {"requested": requested, "supported": [MODERN_VERSION]}
        }
    })
}

/// What the guard does with one inbound line. Public: this *is*
/// the era-admission seam, and in-repo consumers (the gauntlet's
/// protocol-skew task) probe it directly.
#[derive(Debug, PartialEq)]
pub enum GuardAction {
    /// Forward these bytes to the SDK, unmodified.
    Forward(Vec<u8>),
    /// Answer it here; the SDK never sees it.
    Respond(Value),
    /// Drop silently (blank line, notification).
    Drop,
}

/// Whether an `id` value is one the SDK can carry: a string, or an
/// integer literal inside `i64`. Anything else (null, bool, float
/// form, array, object, or an integer beyond `i64`) is refused by
/// [`classify`] with the hand-rolled server's `-32600` shape.
fn id_is_carryable(id: &Value) -> bool {
    match id {
        Value::String(_) => true,
        Value::Number(number) => number.as_i64().is_some() && is_integer_text(number),
        _ => false,
    }
}

/// Classify one line. Pure: unit-tested without any transport.
pub fn classify(line: &[u8]) -> GuardAction {
    let text = String::from_utf8_lossy(line);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return GuardAction::Drop;
    }
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(error) => {
            return GuardAction::Respond(error_frame(
                Value::Null,
                PARSE_ERROR,
                &format!("parse error: {error}"),
            ));
        }
    };
    let Some(object) = value.as_object() else {
        return GuardAction::Respond(error_frame(
            Value::Null,
            INVALID_REQUEST,
            "request must be a JSON object",
        ));
    };
    // No `id` key: a notification. Dropped silently — and never
    // executed (the fleet contract; this surface carries the
    // write-class `flow_run`).
    if !object.contains_key("id") {
        return GuardAction::Drop;
    }
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    // Envelope validity, with the hand-rolled server's refusal
    // shape: -32600, null id, one message for every malformed
    // envelope (bad/missing jsonrpc marker, non-string method,
    // uncarryable id).
    let method = object.get("method").and_then(Value::as_str);
    let jsonrpc_ok = object.get("jsonrpc").and_then(Value::as_str) == Some("2.0");
    if !jsonrpc_ok || method.is_none() || !id_is_carryable(&id) {
        return GuardAction::Respond(error_frame(
            Value::Null,
            INVALID_REQUEST,
            "Invalid JSON-RPC request",
        ));
    }
    let method = method.unwrap_or_default();
    // Params, when present, must be an object (the hand-rolled
    // dispatch checked this before any method handling; a present
    // `null` is not an object either).
    if let Some(params) = object.get("params")
        && !params.is_object()
    {
        return GuardAction::Respond(error_frame(id, INVALID_PARAMS, "Params must be an object"));
    }
    if method == "initialize" {
        let requested = object
            .get("params")
            .and_then(|p| p.get("protocolVersion"))
            .and_then(Value::as_str)
            .map_or(Value::Null, |v| Value::String(v.to_string()));
        return GuardAction::Respond(unsupported_version_frame(id, requested));
    }
    // Strict tools/call params (GAP-2): the SDK's typed params
    // would silently drop unknown keys; the hand-rolled server
    // refused them, so the refusal lives here. The modern era's
    // multi-round-trip retry fields are accepted and ignored,
    // exactly as before (this server never returns
    // `input_required`, so no retry can ever be meaningful).
    if method == "tools/call"
        && let Some(params) = object.get("params").and_then(Value::as_object)
        && params.keys().any(|key| {
            !matches!(
                key.as_str(),
                "name" | "arguments" | "_meta" | "inputResponses" | "requestState"
            )
        })
    {
        return GuardAction::Respond(error_frame(
            id,
            INVALID_PARAMS,
            "Unknown tools/call parameters",
        ));
    }
    GuardAction::Forward(trimmed.as_bytes().to_vec())
}

async fn write_frame(stdout: &SharedStdout, frame: &Value) {
    let mut line = serde_json::to_string(frame).unwrap_or_default();
    line.push('\n');
    let mut out = stdout.lock().await;
    let _ = out.write_all(line.as_bytes()).await;
    let _ = out.flush().await;
}

async fn write_bytes(stdout: &SharedStdout, bytes: &[u8]) {
    let mut out = stdout.lock().await;
    let _ = out.write_all(bytes).await;
    let _ = out.flush().await;
}

/// One inbound line, bounded: the error trips when the content
/// (excluding the newline) reaches [`MAX_LINE_BYTES`]; an oversized
/// line's remainder is drained.
enum LineOutcome {
    Line(Vec<u8>),
    Oversized,
    Eof,
}

async fn next_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<LineOutcome> {
    let mut out: Vec<u8> = Vec::new();
    let mut content_len = 0usize;
    let mut oversized = false;
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok(if oversized {
                LineOutcome::Oversized
            } else if out.is_empty() && content_len == 0 {
                LineOutcome::Eof
            } else {
                LineOutcome::Line(out)
            });
        }
        match chunk.iter().position(|&b| b == b'\n') {
            Some(pos) => {
                content_len += pos;
                if content_len >= MAX_LINE_BYTES {
                    oversized = true;
                }
                if !oversized {
                    out.extend_from_slice(&chunk[..pos]);
                }
                reader.consume(pos + 1);
                return Ok(if oversized {
                    LineOutcome::Oversized
                } else {
                    LineOutcome::Line(out)
                });
            }
            None => {
                let len = chunk.len();
                content_len += len;
                if content_len >= MAX_LINE_BYTES {
                    oversized = true;
                    out.clear();
                } else {
                    out.extend_from_slice(chunk);
                }
                reader.consume(len);
            }
        }
    }
}

/// Run the guard until stdin reaches EOF, then shut the duplex
/// write side down: a split write half signals end-of-input to the
/// SDK service only through an explicit shutdown, never by being
/// dropped (both halves share ownership of the stream).
pub async fn run(
    stdin: Stdin,
    mut to_server: WriteHalf<DuplexStream>,
    stdout: SharedStdout,
    ordering: Arc<Ordering>,
) -> std::io::Result<()> {
    let result = forward_loop(stdin, &mut to_server, &stdout, &ordering).await;
    let _ = to_server.shutdown().await;
    result
}

async fn forward_loop(
    stdin: Stdin,
    to_server: &mut WriteHalf<DuplexStream>,
    stdout: &SharedStdout,
    ordering: &Ordering,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stdin);
    loop {
        match next_line(&mut reader).await? {
            LineOutcome::Eof => return Ok(()),
            LineOutcome::Oversized => {
                ordering.wait_drained().await;
                write_frame(
                    stdout,
                    &error_frame(
                        Value::Null,
                        PARSE_ERROR,
                        &format!("frame exceeds the {MAX_LINE_BYTES}-byte MCP cap"),
                    ),
                )
                .await;
            }
            LineOutcome::Line(bytes) => match classify(&bytes) {
                GuardAction::Drop => {}
                GuardAction::Respond(frame) => {
                    ordering.wait_drained().await;
                    write_frame(stdout, &frame).await;
                }
                GuardAction::Forward(mut line) => {
                    if let Ok(frame) = serde_json::from_slice::<Value>(&line)
                        && let Some(id) = frame.get("id").cloned()
                    {
                        ordering.track(id).await;
                    }
                    line.push(b'\n');
                    to_server.write_all(&line).await?;
                    to_server.flush().await?;
                }
            },
        }
    }
}

/// Copy SDK responses from the duplex to stdout, one line at a time,
/// marking each answered request id complete for the guard's
/// ordering wait.
pub async fn pump(
    mut from_server: ReadHalf<DuplexStream>,
    stdout: SharedStdout,
    ordering: Arc<Ordering>,
) {
    let mut reader = BufReader::new(&mut from_server);
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                write_bytes(&stdout, &line).await;
                if let Ok(frame) = serde_json::from_slice::<Value>(&line)
                    && let Some(id) = frame.get("id")
                {
                    ordering.complete(id).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_str(s: &str) -> GuardAction {
        classify(s.as_bytes())
    }

    #[test]
    fn blank_lines_and_notifications_drop() {
        assert_eq!(classify_str("  \n"), GuardAction::Drop);
        assert_eq!(
            classify_str("{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}"),
            GuardAction::Drop
        );
        // Even a notification-shaped initialize is a notification.
        assert_eq!(
            classify_str("{\"jsonrpc\":\"2.0\",\"method\":\"initialize\",\"params\":{}}"),
            GuardAction::Drop
        );
        // A notification-shaped tools/call drops unexecuted (the
        // fleet contract; write-class tools on this surface).
        assert_eq!(
            classify_str(
                "{\"jsonrpc\":\"2.0\",\"method\":\"tools/call\",\"params\":{\"name\":\"flow_run\",\"arguments\":{\"task\":\"x\"}}}"
            ),
            GuardAction::Drop
        );
    }

    #[test]
    fn framing_errors_follow_the_fleet_contract() {
        match classify_str("this is not json") {
            GuardAction::Respond(f) => {
                assert_eq!(f["error"]["code"], PARSE_ERROR);
                assert_eq!(f["id"], Value::Null);
                let message = f["error"]["message"].as_str().expect("message");
                assert!(message.starts_with("parse error: "), "{message}");
            }
            other => panic!("expected respond, got {other:?}"),
        }
        match classify_str("[1,2,3]") {
            GuardAction::Respond(f) => assert_eq!(f["error"]["code"], INVALID_REQUEST),
            other => panic!("expected respond, got {other:?}"),
        }
    }

    #[test]
    fn initialize_is_refused_with_supported_versions() {
        for version in ["2024-11-05", "2025-06-18", "2025-11-25", "2026-07-28"] {
            let frame = format!(
                "{{\"id\":7,\"jsonrpc\":\"2.0\",\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{version}\"}}}}"
            );
            match classify_str(&frame) {
                GuardAction::Respond(f) => {
                    assert_eq!(f["id"], 7);
                    assert_eq!(f["error"]["code"], UNSUPPORTED_PROTOCOL_VERSION);
                    assert_eq!(f["error"]["data"]["requested"], version);
                    assert_eq!(f["error"]["data"]["supported"], json!(["2026-07-28"]));
                }
                other => panic!("expected respond for {version}, got {other:?}"),
            }
        }
        // An initialize without a version still refuses, naming
        // no requested version.
        match classify_str("{\"id\":8,\"jsonrpc\":\"2.0\",\"method\":\"initialize\",\"params\":{}}")
        {
            GuardAction::Respond(f) => {
                assert_eq!(f["error"]["code"], UNSUPPORTED_PROTOCOL_VERSION);
                assert_eq!(f["error"]["data"]["requested"], Value::Null);
            }
            other => panic!("expected respond, got {other:?}"),
        }
    }

    #[test]
    fn invalid_ids_are_refused_with_the_legacy_shape() {
        // The hand-rolled server's refusal: -32600, null id,
        // "Invalid JSON-RPC request" — for every id the SDK
        // cannot carry, including >i64 integers (GAP-1 fallback).
        for frame in [
            "{\"id\":null,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{}}",
            "{\"id\":true,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{}}",
            "{\"id\":1.5,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{}}",
            "{\"id\":1e3,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{}}",
            "{\"id\":[1],\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{}}",
            "{\"id\":1180591620717411303424,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{}}",
            "{\"id\":-1180591620717411303424,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{}}",
        ] {
            match classify_str(frame) {
                GuardAction::Respond(f) => {
                    assert_eq!(f["id"], Value::Null, "{frame}");
                    assert_eq!(f["error"]["code"], INVALID_REQUEST, "{frame}");
                    assert_eq!(f["error"]["message"], "Invalid JSON-RPC request", "{frame}");
                }
                other => panic!("expected respond for {frame}, got {other:?}"),
            }
        }
        // String and in-range integer ids forward.
        for frame in [
            "{\"id\":\"abc\",\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{}}",
            "{\"id\":-7,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{}}",
            "{\"id\":9223372036854775807,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{}}",
        ] {
            assert!(
                matches!(classify_str(frame), GuardAction::Forward(_)),
                "{frame}"
            );
        }
    }

    #[test]
    fn malformed_envelopes_are_refused() {
        for frame in [
            "{\"id\":1,\"method\":\"tools/list\",\"params\":{}}",
            "{\"id\":1,\"jsonrpc\":\"1.0\",\"method\":\"tools/list\",\"params\":{}}",
            "{\"id\":1,\"jsonrpc\":\"2.0\",\"params\":{}}",
            "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":42,\"params\":{}}",
        ] {
            match classify_str(frame) {
                GuardAction::Respond(f) => {
                    assert_eq!(f["error"]["code"], INVALID_REQUEST, "{frame}");
                    assert_eq!(f["id"], Value::Null, "{frame}");
                }
                other => panic!("expected respond for {frame}, got {other:?}"),
            }
        }
    }

    #[test]
    fn non_object_params_are_refused_before_dispatch() {
        for frame in [
            "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":[1]}",
            "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":null}",
            "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"tools/call\",\"params\":\"x\"}",
        ] {
            match classify_str(frame) {
                GuardAction::Respond(f) => {
                    assert_eq!(f["error"]["code"], INVALID_PARAMS, "{frame}");
                    assert_eq!(f["error"]["message"], "Params must be an object", "{frame}");
                    assert_eq!(f["id"], 1, "{frame}");
                }
                other => panic!("expected respond for {frame}, got {other:?}"),
            }
        }
    }

    #[test]
    fn strict_tools_call_params_are_refused() {
        let frame = "{\"id\":3,\"jsonrpc\":\"2.0\",\"method\":\"tools/call\",\"params\":{\"name\":\"flow_status\",\"arguments\":{},\"bogus\":1}}";
        match classify_str(frame) {
            GuardAction::Respond(f) => {
                assert_eq!(f["id"], 3);
                assert_eq!(f["error"]["code"], INVALID_PARAMS);
                assert_eq!(f["error"]["message"], "Unknown tools/call parameters");
            }
            other => panic!("expected respond, got {other:?}"),
        }
        // The modern retry fields are accepted (and ignored).
        let frame = "{\"id\":3,\"jsonrpc\":\"2.0\",\"method\":\"tools/call\",\"params\":{\"name\":\"flow_status\",\"arguments\":{},\"inputResponses\":{},\"requestState\":\"s\"}}";
        assert!(matches!(classify_str(frame), GuardAction::Forward(_)));
    }

    #[test]
    fn modern_frames_forward_verbatim() {
        for frame in [
            "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"server/discover\",\"params\":{\"_meta\":{}}}",
            "{\"id\":2,\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"params\":{\"_meta\":{}}}",
            "{\"id\":3,\"jsonrpc\":\"2.0\",\"method\":\"tools/call\",\"params\":{\"name\":\"flow_status\",\"arguments\":{},\"_meta\":{}}}",
            "{\"id\":4,\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"params\":{\"_meta\":{}}}",
        ] {
            match classify_str(frame) {
                GuardAction::Forward(bytes) => assert_eq!(bytes, frame.as_bytes()),
                other => panic!("expected forward for {frame}, got {other:?}"),
            }
        }
    }
}
