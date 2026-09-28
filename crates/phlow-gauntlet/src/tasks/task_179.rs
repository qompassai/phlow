// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Task 179 — bridge stdio framing (rust, V).
//!
//! The seam is the bridge's stdio byte layer. Framing is exact:
//! partial reads buffer until a complete newline-delimited JSON-RPC
//! message arrives, a peer that closes stdout mid-message yields the
//! typed [`BridgeError::Eof`](crate::bridge::BridgeError::Eof) (the
//! in-flight request fails typed and the bridge exits 0 — clean, not a
//! crash), and malformed frames are typed errors. The bridge never
//! desyncs silently.
//!
//! Declared framing contract (also in `crate::bridge` docs):
//! newline-delimited JSON-RPC 2.0, one message per line, `\n`
//! terminator, UTF-8, at most `MAX_MESSAGE_BYTES` per line.

use crate::bridge::{
    BridgeError, LoopReport, MAX_MESSAGE_BYTES, PeerEvent, StdioFramer, run_bridge_loop,
};
use crate::skillopt::driver::{CaseReport, TaskDriverError};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Task id.
pub const ID: &str = "task-179";
/// Task name.
pub const NAME: &str = "bridge stdio framing";
/// Task kind.
pub const KIND: TaskKind = TaskKind::Rust;
/// Driver cases: 3 validation.
pub const CASES: [&str; 3] = [
    "split_message_reassembled_once",
    "eof_mid_message_typed",
    "oversize_frame_rejected",
];

/// V1: one JSON-RPC message split across 5 writes → reassembled
/// exactly once, handler invoked once. A second drain delivers
/// nothing: no duplication, no residue.
fn case_split_message_reassembled_once() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let message =
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"browser_evaluate"}}"#;
    let mut wire = message.as_bytes().to_vec();
    wire.push(b'\n');
    // Five uneven chunks; the newline lands mid-chunk-4.
    let cuts = [11usize, 29, 47, 68, wire.len()];
    let mut framer = StdioFramer::new();
    let mut handler_invocations: u64 = 0;
    let mut delivered: Vec<serde_json::Value> = Vec::new();
    let mut prev = 0;
    for &cut in &cuts {
        let messages = framer.push_chunk(&wire[prev..cut]).map_err(fixture_err)?;
        for m in messages {
            handler_invocations += 1;
            delivered.push(m);
        }
        prev = cut;
    }
    if delivered.len() != 1 {
        failures.push(format!("delivered {} messages, want 1", delivered.len()));
    }
    if handler_invocations != 1 {
        failures.push(format!("handler invoked {handler_invocations}x, want 1"));
    }
    if delivered
        .first()
        .map(|v| v == &serde_json::from_str::<serde_json::Value>(message).unwrap())
        != Some(true)
    {
        failures.push("reassembled bytes differ from the sent message".to_string());
    }
    if framer.pending_bytes() != 0 {
        failures.push(format!("{} residue bytes left", framer.pending_bytes()));
    }
    // A second drain delivers nothing.
    let again = framer.push_chunk(&[]).map_err(fixture_err)?;
    if !again.is_empty() {
        failures.push("second drain duplicated the message".to_string());
    }
    evidence.push(format!(
        "5 chunks -> 1 message, handler x1, residue {} bytes",
        framer.pending_bytes()
    ));
    evidence.push("contract: newline-delimited JSON-RPC 2.0, \\n terminator".to_string());
    finish_case(
        CASES[0],
        failures,
        evidence,
        serde_json::json!({
            "chunks": 5,
            "messages": delivered.len(),
            "handler_invocations": handler_invocations,
        }),
    )
}

/// V2: the peer closes stdout mid-message → `BridgeError::Eof`, the
/// in-flight request fails typed, and the bridge loop exits 0 (clean,
/// not a crash).
fn case_eof_mid_message_typed() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    // Direct framer check: partial message, then finish().
    let mut framer = StdioFramer::new();
    let partial = br#"{"jsonrpc":"2.0","id":1,"method":"initial"#;
    let messages = framer.push_chunk(partial).map_err(fixture_err)?;
    if !messages.is_empty() {
        failures.push("partial message was delivered".to_string());
    }
    match framer.finish() {
        Err(BridgeError::Eof) => evidence.push("finish() -> BridgeError::Eof (typed)".to_string()),
        other => failures.push(format!("finish() gave {other:?}, want Err(Eof)")),
    }
    // Full loop: chunks, then the peer vanishes mid-message.
    let report: LoopReport = run_bridge_loop(vec![
        PeerEvent::Chunk(b"{\"jsonrpc\":".to_vec()),
        PeerEvent::Chunk(b"\"2.0\",\"id\":1}\n".to_vec()),
        PeerEvent::Chunk(br#"{"jsonrpc":"2.0","id":2"#.to_vec()),
        PeerEvent::Eof,
    ]);
    if report.handled != 1 {
        failures.push(format!("loop handled {} messages, want 1", report.handled));
    }
    if report.exit_code != 0 {
        failures.push(format!("loop exit code {}, want 0", report.exit_code));
    }
    if report.inflight_failed != vec![1u64] {
        failures.push(format!(
            "in-flight failures {:?}, want [1] typed as Eof",
            report.inflight_failed
        ));
    }
    if report.eof_clean {
        failures.push("eof_clean must be false: a partial message was pending".to_string());
    }
    evidence.push(format!(
        "loop: handled={}, exit_code={}, inflight_failed={:?} (typed Eof)",
        report.handled, report.exit_code, report.inflight_failed
    ));
    // And the clean path still works: no partial message at EOF.
    let clean: LoopReport = run_bridge_loop(vec![
        PeerEvent::Chunk(b"{\"jsonrpc\":\"2.0\",\"id\":1}\n".to_vec()),
        PeerEvent::Eof,
    ]);
    if clean.exit_code != 0 || !clean.eof_clean || !clean.inflight_failed.is_empty() {
        failures.push(format!(
            "clean EOF wrong: exit={} eof_clean={} failed={:?}",
            clean.exit_code, clean.eof_clean, clean.inflight_failed
        ));
    }
    evidence.push("clean EOF: exit 0, eof_clean=true, nothing failed".to_string());
    finish_case(
        CASES[1],
        failures,
        evidence,
        serde_json::json!({
            "exit_code": report.exit_code,
            "handled": report.handled,
            "inflight_failed": report.inflight_failed,
        }),
    )
}

/// A line longer than `MAX_MESSAGE_BYTES` with no newline is a typed
/// `FramingTooLarge` error — the bridge refuses the frame rather than
/// buffering it forever.
fn case_oversize_frame_rejected() -> Result<CaseReport, TaskDriverError> {
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    let mut framer = StdioFramer::new();
    let big = vec![b'x'; MAX_MESSAGE_BYTES + 1];
    match framer.push_chunk(&big) {
        Err(BridgeError::FramingTooLarge { bytes }) => {
            evidence.push(format!("FramingTooLarge{{bytes: {bytes}}} (typed)"));
            if bytes != MAX_MESSAGE_BYTES + 1 {
                failures.push(format!("reported {bytes} bytes, sent {}", big.len()));
            }
        }
        other => failures.push(format!("oversize frame gave {other:?}")),
    }
    if framer.pending_bytes() != 0 {
        failures.push("framer kept the oversize bytes (desync risk)".to_string());
    }
    // The framer recovers: a good message right after still parses.
    let ok = framer
        .push_chunk(b"{\"jsonrpc\":\"2.0\",\"id\":3}\n")
        .map_err(fixture_err)?;
    if ok.len() != 1 {
        failures.push("framer did not recover after FramingTooLarge".to_string());
    }
    evidence.push("framer recovered: next message parsed exactly once".to_string());
    finish_case(
        CASES[2],
        failures,
        evidence,
        serde_json::json!({
            "max_message_bytes": MAX_MESSAGE_BYTES,
            "rejected_bytes": big.len(),
        }),
    )
}

fn fixture_err(e: BridgeError) -> TaskDriverError {
    TaskDriverError::Fixture {
        what: "framing".to_string(),
        detail: format!("task-179: {e}"),
    }
}

fn finish_case(
    case: &'static str,
    failures: Vec<String>,
    mut evidence: Vec<String>,
    metrics: serde_json::Value,
) -> Result<CaseReport, TaskDriverError> {
    evidence.extend(failures.iter().cloned());
    let mut report = CaseReport::pass(case, metrics, evidence);
    report.passed = failures.is_empty();
    report.failures = failures;
    Ok(report)
}

/// Run one driver case by name.
pub fn run_case(case: &str) -> Result<CaseReport, TaskDriverError> {
    match case {
        "split_message_reassembled_once" => case_split_message_reassembled_once(),
        "eof_mid_message_typed" => case_eof_mid_message_typed(),
        "oversize_frame_rejected" => case_oversize_frame_rejected(),
        _ => Err(TaskDriverError::Fixture {
            what: "case".to_string(),
            detail: format!("task-179: unknown case '{case}'"),
        }),
    }
}

/// Task-level entry for the gauntlet runner: the headline case — the
/// split-message reassembly itself.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    match run_case(CASES[0]) {
        Ok(r) if r.passed => TaskOutcome::Pass {
            evidence: r.evidence,
        },
        Ok(r) => TaskOutcome::Fail {
            where_: "task-179".to_string(),
            how: r.failures.join("; "),
            evidence: r.evidence,
        },
        Err(e) => TaskOutcome::Fail {
            where_: "task-179".to_string(),
            how: e.to_string(),
            evidence: vec![],
        },
    }
}
