//! Integration tests for task-145 (exact submission payload
//! preview).
//!
//! Four driver cases — 2 validation, 2 adversarial — against scripted
//! fixtures (MOCK) with the scaffold's `FakePlatform` as the platform
//! double: the preview bytes equal the serializer output byte-for-byte
//! and the identical bytes reach the platform; the envelope carries
//! program id, submission id, and the content hash; mutating the
//! finding after preview marks it stale and the gate refuses the
//! mutated bytes until re-preview; preview for a non-`Approved` finding
//! is a typed refusal.

use phlow_gauntlet::TaskKind;
use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_145;

fn check_case(case: &str) -> CaseReport {
    let report = task_145::run_case(case)
        .unwrap_or_else(|e| panic!("task-145 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-145 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: metadata contract pins the task; preview == serializer output
/// byte-for-byte, and the wire bytes are the preview bytes — no hidden
/// mutation between preview and submit.
#[test]
fn preview_equals_serializer_output() {
    assert_eq!(task_145::ID, "task-145");
    assert_eq!(task_145::NAME, "submission-payload-preview");
    assert_eq!(task_145::KIND, TaskKind::Rust);
    let report = check_case("preview_equals_serializer_output");
    let m = &report.metrics;
    assert!(m["preview_bytes"].as_u64().unwrap() > 0);
    assert!(m["wire_matches_preview"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("byte-for-byte"),
        "evidence must state the byte equality:\n{joined}"
    );
}

/// V2: the envelope carries program id, submission id, and the content
/// hash binding it to the exact report bytes.
#[test]
fn preview_carries_envelope() {
    let report = check_case("preview_carries_envelope");
    let m = &report.metrics;
    assert!(
        m["content_hash"].as_str().map(|s| s.len()).unwrap_or(0) == 64,
        "the envelope must carry the content hash"
    );
}

// --- adversarial ---

/// A1: mutating the finding after preview marks the preview stale, the
/// gate `HashMismatch`es the mutated bytes, and only a re-preview
/// authorizes the send.
#[test]
fn mutation_invalidates_preview() {
    let report = check_case("mutation_invalidates_preview");
    let m = &report.metrics;
    assert!(m["stale_detected"].as_bool().unwrap());
    assert!(m["hash_mismatch"].as_bool().unwrap());
    assert!(m["repreview_authorized"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("HashMismatch"),
        "evidence must name the gate's refusal:\n{joined}"
    );
}

/// A2: preview is part of the approval flow, not a side door —
/// `Reportable` and `Candidate` findings are refused with the typed
/// `NotApproved`.
#[test]
fn unapproved_preview_refused() {
    let report = check_case("unapproved_preview_refused");
    let m = &report.metrics;
    let refused: Vec<&str> = m["refused_states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(refused, ["Reportable", "Candidate"]);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("NotApproved"),
        "evidence must name the typed refusal:\n{joined}"
    );
}
