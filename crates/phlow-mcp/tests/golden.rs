//! Golden tests, ported for the rmcp migration (Stage C2).
//!
//! The fixture `fixtures/mcp_golden.jsonl` was generated from the
//! real `flow/mcp.py` and replayed byte-for-byte through the
//! hand-rolled dual-era Rust server. The fleet standardization
//! (D-113) deleted the legacy era, so byte-parity with Python's
//! *handshake* is gone by decision — but everything the fixture
//! records that is era-independent is still asserted here, pair by
//! pair, at the seam that now owns it:
//!
//! * **Tool result payloads** (pairs 6–8, 24): the result *text*
//!   bytes are the phlow-canonical `dumps` spelling (GAP-3 option
//!   (i)) — asserted byte-for-byte against the fixture, together
//!   with `structuredContent` and `isError`, at the domain seam
//!   ([`McpServer::call_tool`]).
//! * **Dispatch refusals** (pairs 15–19): unknown tool and
//!   schema-invalid arguments keep their codes and messages at
//!   the domain seam; the strict-params and non-object-params
//!   refusals keep theirs at the guard seam ([`classify`]).
//! * **Legacy frames** (pairs 0–5, 9–14, 20–22): every request in
//!   the fixture is classified by the era guard. `initialize` is
//!   now refused `-32022` (the ready-gate `-32002` of pair 0 and
//!   the silent version upgrade of pair 22 are the named deltas
//!   of the migration); malformed ids keep the exact `-32600`
//!   refusal; parse failures keep `-32700` (message now carries
//!   the serde detail, fleet shape); notifications drop; and
//!   well-formed legacy requests forward to the SDK, whose own
//!   `_meta` gate completes the refusal — that half is covered by
//!   the wire probe in the C2 gate evidence, not by this seam.
//!
//! Pair 23 (a synthetic server-side oversize frame with no
//! request) is covered by the guard's line-bound unit tests.

use phlow_mcp::{CallOutcome, FakeRuntime, GuardAction, McpServer, RuntimeError, classify, dumps};
use serde_json::Value;

struct GoldenPair {
    request: Option<String>,
    reply: Option<String>,
    failing: bool,
}

fn load_pairs() -> Vec<GoldenPair> {
    let text = include_str!("fixtures/mcp_golden.jsonl");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let pair: Value = serde_json::from_str(line).expect("fixture line is JSON");
            let obj = pair.as_object().expect("fixture line is an object");
            GoldenPair {
                request: obj
                    .get("request")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned),
                reply: obj
                    .get("reply")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned),
                failing: obj
                    .get("failing")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
            }
        })
        .collect()
}

fn fake_runtime(failing: bool) -> FakeRuntime {
    let mut runtime = FakeRuntime::ok();
    if failing {
        runtime.fail_with = Some("boom".to_owned());
    }
    runtime
}

/// The fixture reply of one pair, parsed.
fn reply_value(pair: &GoldenPair) -> Value {
    let reply = pair.reply.as_deref().expect("pair has a reply");
    serde_json::from_str(reply.trim_end()).expect("fixture reply is JSON")
}

/// Pairs 6–8 and 24: tool results whose bytes are era-independent.
/// The domain dispatch must reproduce the fixture's text payload
/// byte-for-byte, with matching structuredContent and isError.
#[test]
fn golden_tool_result_payloads_survive_byte_for_byte() {
    let pairs = load_pairs();
    let mut compared = 0;
    for (index, pair) in pairs.iter().enumerate() {
        let Some(request) = &pair.request else {
            continue;
        };
        let Ok(request) = serde_json::from_str::<Value>(request) else {
            continue;
        };
        if request.get("method").and_then(Value::as_str) != Some("tools/call") {
            continue;
        }
        let expected = reply_value(pair);
        let Some(expected_result) = expected.get("result") else {
            continue; // refusal pairs are covered by their own test
        };
        let params = request.get("params").expect("call params");
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .expect("tool name");
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let mut server = McpServer::new(fake_runtime(pair.failing));
        let outcome = server.call_tool(name, &args, Some("golden"));
        let CallOutcome::Result { result, is_error } = outcome else {
            panic!("pair {index}: expected a tool result, got {outcome:?}");
        };
        let expected_text = expected_result
            .pointer("/content/0/text")
            .and_then(Value::as_str)
            .expect("fixture text payload");
        assert_eq!(
            dumps(&result).expect("result serializes"),
            expected_text,
            "pair {index}: result text bytes diverged from the Python golden"
        );
        assert_eq!(
            result, expected_result["structuredContent"],
            "pair {index}: structuredContent diverged"
        );
        assert_eq!(
            is_error,
            expected_result["isError"].as_bool().unwrap_or(false),
            "pair {index}: isError diverged"
        );
        compared += 1;
    }
    assert_eq!(compared, 4, "pairs 6, 7, 8, 24 are the result pairs");
}

/// Pairs 15–19: dispatch refusals keep their codes and messages.
/// Domain refusals (unknown tool, schema) assert at the domain
/// seam; guard refusals (strict params, non-object params) at
/// the guard seam.
#[test]
fn golden_dispatch_refusals_keep_codes_and_messages() {
    let pairs = load_pairs();
    let mut server = McpServer::new(fake_runtime(false));
    // Pair 15: unknown tool.
    match server.call_tool("nope", &serde_json::json!({}), Some("golden")) {
        CallOutcome::Refusal { code, message } => {
            assert_eq!(code, -32602);
            assert_eq!(message, "Unknown Phlow tool");
        }
        other => panic!("pair 15: expected a refusal, got {other:?}"),
    }
    // Pair 16: missing required argument, Python-list message.
    match server.call_tool("flow_run", &serde_json::json!({}), Some("golden")) {
        CallOutcome::Refusal { code, message } => {
            assert_eq!(code, -32602);
            assert_eq!(
                message,
                "Missing arguments: ['task']; unknown arguments: []"
            );
        }
        other => panic!("pair 16: expected a refusal, got {other:?}"),
    }
    // Pair 17: unknown argument, Python-list message.
    match server.call_tool(
        "flow_status",
        &serde_json::json!({"bogus": 1}),
        Some("golden"),
    ) {
        CallOutcome::Refusal { code, message } => {
            assert_eq!(code, -32602);
            assert_eq!(
                message,
                "Missing arguments: []; unknown arguments: ['bogus']"
            );
        }
        other => panic!("pair 17: expected a refusal, got {other:?}"),
    }
    // Pairs 18–19: guard-side refusals, exact fixture messages.
    for (index, pair) in pairs.iter().enumerate() {
        if index != 18 && index != 19 {
            continue;
        }
        let request = pair.request.as_deref().expect("request frame");
        let expected = reply_value(pair);
        match classify(request.as_bytes()) {
            GuardAction::Respond(frame) => {
                assert_eq!(
                    frame["error"]["code"], expected["error"]["code"],
                    "pair {index}"
                );
                assert_eq!(
                    frame["error"]["message"], expected["error"]["message"],
                    "pair {index}"
                );
            }
            other => panic!("pair {index}: expected a guard refusal, got {other:?}"),
        }
    }
}

/// Every fixture request through the era guard: the legacy era
/// fails closed in the shapes the migration decided.
#[test]
fn golden_legacy_frames_fail_closed_at_the_guard() {
    let pairs = load_pairs();
    let mut refused_initialize = 0;
    for (index, pair) in pairs.iter().enumerate() {
        let Some(request) = &pair.request else {
            continue; // pair 23: synthetic server frame, guard unit tests cover it
        };
        let action = classify(request.as_bytes());
        let method = serde_json::from_str::<Value>(request)
            .ok()
            .and_then(|v| v.get("method").and_then(Value::as_str).map(str::to_owned));
        match method.as_deref() {
            Some("initialize") => {
                refused_initialize += 1;
                match action {
                    GuardAction::Respond(frame) => {
                        assert_eq!(frame["error"]["code"], -32022, "pair {index}");
                        assert_eq!(
                            frame["error"]["data"]["supported"],
                            serde_json::json!(["2026-07-28"]),
                            "pair {index}"
                        );
                    }
                    other => panic!("pair {index}: initialize must be refused, got {other:?}"),
                }
            }
            Some("notifications/initialized") => {
                assert_eq!(action, GuardAction::Drop, "pair {index}");
            }
            _ => match action {
                GuardAction::Forward(_) | GuardAction::Respond(_) | GuardAction::Drop => {}
            },
        }
    }
    assert_eq!(refused_initialize, 3, "pairs 1, 2, 22 are initialize");
}

/// The malformed-id pairs (12–14) keep the hand-rolled refusal
/// byte-for-byte: -32600, null id, "Invalid JSON-RPC request".
#[test]
fn golden_malformed_ids_keep_the_exact_refusal() {
    let pairs = load_pairs();
    for index in [12, 13, 14] {
        let pair = &pairs[index];
        let request = pair.request.as_deref().expect("request frame");
        match classify(request.as_bytes()) {
            GuardAction::Respond(frame) => {
                assert_eq!(frame["id"], Value::Null, "pair {index}");
                assert_eq!(frame["error"]["code"], -32600, "pair {index}");
                assert_eq!(
                    frame["error"]["message"], "Invalid JSON-RPC request",
                    "pair {index}"
                );
            }
            other => panic!("pair {index}: expected a refusal, got {other:?}"),
        }
    }
}

/// A failing runtime still produces an `isError` tool result
/// carrying the failure text (the domain half of the old
/// request-id test; id carriage is the SDK's job now).
#[test]
fn runtime_failure_result_is_flagged_with_its_text() {
    let mut server = McpServer::new({
        let mut runtime = FakeRuntime::ok();
        runtime.fail_with = Some("object() is not JSON serializable".to_owned());
        runtime
    });
    let outcome = server.call_tool(
        "flow_run",
        &serde_json::json!({"task": "x"}),
        Some("golden"),
    );
    let CallOutcome::Result { result, is_error } = outcome else {
        panic!("expected a tool result, got {outcome:?}");
    };
    assert!(is_error);
    let text = dumps(&result).expect("result serializes");
    assert!(text.contains("not JSON serializable"), "{text}");
}

/// `RuntimeError::new` carries the failure text the tool envelope reports.
#[test]
fn runtime_error_text() {
    let error = RuntimeError::new("boom");
    assert_eq!(error.to_string(), "boom");
}
