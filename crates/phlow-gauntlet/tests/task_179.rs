// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-179 (bridge stdio framing).
//!
//! Three validation cases against the scripted stdio peer (chunked
//! writes, abrupt EOF, oversize line): a message split across 5 writes
//! is reassembled exactly once with the handler invoked once; a peer
//! that closes stdout mid-message yields the typed BridgeError::Eof,
//! fails the in-flight request typed, and the bridge loop exits 0.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_179;

fn check_case(case: &str) -> CaseReport {
    let report = task_179::run_case(case)
        .unwrap_or_else(|e| panic!("task-179 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-179 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: 5 chunks -> 1 message, handler invoked exactly once, no
/// residue, no duplication on re-drain.
#[test]
fn split_message_reassembled_once() {
    assert_eq!(task_179::ID, "task-179");
    let report = check_case("split_message_reassembled_once");
    let m = &report.metrics;
    assert_eq!(m["chunks"].as_u64().unwrap(), 5);
    assert_eq!(m["messages"].as_u64().unwrap(), 1);
    assert_eq!(m["handler_invocations"].as_u64().unwrap(), 1);
}

/// V2: mid-message EOF -> typed Eof, in-flight request [1] failed
/// typed, loop exit code 0 (clean, not a crash).
#[test]
fn eof_mid_message_typed() {
    let report = check_case("eof_mid_message_typed");
    let m = &report.metrics;
    assert_eq!(m["exit_code"].as_i64().unwrap(), 0);
    assert_eq!(m["handled"].as_u64().unwrap(), 1);
    assert_eq!(m["inflight_failed"].as_array().unwrap().len(), 1);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("BridgeError::Eof"),
        "evidence must name the typed error:\n{joined}"
    );
}

/// An oversize line is a typed FramingTooLarge refusal and the framer
/// recovers for the next message.
#[test]
fn oversize_frame_rejected() {
    let report = check_case("oversize_frame_rejected");
    let m = &report.metrics;
    assert_eq!(
        m["rejected_bytes"].as_u64().unwrap(),
        m["max_message_bytes"].as_u64().unwrap() + 1
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("FramingTooLarge"),
        "evidence must name the typed refusal:\n{joined}"
    );
}
