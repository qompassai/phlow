//! Integration tests for task-83 (streaming vs non-streaming parity).
//!
//! The seam is ABSENT: the provider client never streams —
//! `build_chat_payload` (crates/phlow-llm/src/payload.rs:69) inserts
//! `"stream": false` into every chat payload (measured on the real
//! builder), so the provider is never asked for a stream. SSE
//! vocabulary scans (`event-stream`, `text/event-stream`, `[DONE]`,
//! `reassemble`, `eventsource`, `event_source`) over phlow-llm/src
//! find zero hits. `LlmTransport::post_chat` delivers one decoded
//! JSON body per call: there is no chunk boundary, no reassembly
//! function, no stream-terminator state machine, and `LlmError` has
//! no `TruncatedStream` variant — the design's byte-exact reassembly
//! and truncated-stream criteria are unrepresentable.
//!
//! Whether phlow-llm should support SSE streaming with byte-exact
//! reassembly and truncated-stream typing is a product decision for
//! Matt — banked, not implemented on gauntlet authority.
//!
//! Four cases — 2 validation, 2 adversarial — each self-checking:
//! cases probe the seam and record measured mechanism evidence; the
//! driver then reports the honest seam failure.

use phlow_gauntlet::tasks::task_83;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-83: cannot build Ctx: {e}"))
}

// --- validation ---

/// V1: streaming is disabled by construction — the real payload
/// builder emits stream=false, so there is no streaming mode to be
/// in parity with. The task-level driver then runs all four cases
/// and reports the honest seam failure: the SSE-streaming product
/// decision is banked in the task-level `how`, not implemented on
/// gauntlet authority.
#[test]
fn streaming_disabled_by_construction() {
    assert_eq!(task_83::ID, "task-83");
    assert_eq!(task_83::NAME, "streaming vs non-streaming parity");
    assert_eq!(task_83::KIND, TaskKind::Rust);
    assert_eq!(task_83::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_83::run_case("streaming_disabled_by_construction")
        .unwrap_or_else(|e| panic!("task-83 case failed to run: {e}"));
    assert!(
        report.passed,
        "stream-false case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["stream"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("payload.rs:69"),
        "evidence must pin the stream=false insertion:\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the SSE-streaming product decision for Matt.
    let (where_, how) = match task_83::run(&ctx()) {
        TaskOutcome::Fail { where_, how, .. } => (where_, how),
        TaskOutcome::Pass { evidence } => panic!(
            "task-83 passed: streaming parity was invented, not found\nevidence: {evidence:?}"
        ),
    };
    assert_eq!(where_, "seam", "task-83 must fail at the seam");
    assert!(
        how.contains("product decision for Matt"),
        "the 'how' must bank the product decision: {how}"
    );
    assert!(
        how.contains("never streams"),
        "the 'how' must name the absent streaming mode: {how}"
    );
}

/// V2: there is no SSE vocabulary in the client — the streaming
/// markers have zero hits in phlow-llm/src.
#[test]
fn no_sse_vocabulary() {
    let report = task_83::run_case("no_sse_vocabulary")
        .unwrap_or_else(|e| panic!("task-83 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-sse case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["sse_vocabulary_hits"], 0);
}

// --- adversarial ---

/// A1: chunk-boundary semantics are unrepresentable — the transport
/// contract delivers one decoded JSON body per call, so a boundary
/// splitting a JSON string escape or a multi-byte UTF-8 sequence
/// cannot occur; there is no reassembly function for the design's
/// byte-exactness criterion to live in.
#[test]
fn chunk_boundary_semantics_unrepresentable() {
    let report = task_83::run_case("chunk_boundary_semantics_unrepresentable")
        .unwrap_or_else(|e| panic!("task-83 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-chunks case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["reassembly_hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no chunk boundary"),
        "evidence must show the contract has no chunks:\n{joined}"
    );
}

/// A2: truncation semantics are unrepresentable — `BoundedBody` is a
/// byte cap, not a terminator check, and `LlmError` has no
/// `TruncatedStream` variant: with stream:false the provider's JSON
/// body IS the completion, so "truncated streams never count as
/// complete" has no error to arrive as.
#[test]
fn truncation_semantics_unrepresentable() {
    let report = task_83::run_case("truncation_semantics_unrepresentable")
        .unwrap_or_else(|e| panic!("task-83 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-truncation case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["truncated_stream_variant"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("byte cap, not a terminator check"),
        "evidence must distinguish the cap from termination:\n{joined}"
    );
}
