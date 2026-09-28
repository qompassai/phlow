//! Integration tests for task-141 (evidence preservation + chain of
//! custody).
//!
//! Four driver cases — 2 validation, 2 adversarial — against scripted
//! fixtures (MOCK tool output): byte-exact seal round-trip with the
//! sha256 recorded; the custody chain grows one entry per handling
//! step; a flipped byte fails custody verification with the typed
//! `EvidenceTampered` and lands the finding in quarantine (never
//! dropped); 100 MiB of tool output is stored bounded at the 10 MiB
//! cap with a marker naming the cap.

use phlow_gauntlet::TaskKind;
use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_141;

fn check_case(case: &str) -> CaseReport {
    let report = task_141::run_case(case)
        .unwrap_or_else(|e| panic!("task-141 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-141 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: metadata contract pins the task; the seal round-trips the tool
/// output bytes exactly and records the sha256.
#[test]
fn byte_exact_roundtrip() {
    assert_eq!(task_141::ID, "task-141");
    assert_eq!(task_141::NAME, "evidence-preservation-custody");
    assert_eq!(task_141::KIND, TaskKind::Rust);
    let report = check_case("byte_exact_roundtrip");
    let m = &report.metrics;
    assert_eq!(
        m["input_bytes"].as_u64().unwrap(),
        m["stored_bytes"].as_u64().unwrap(),
        "stored bytes must equal input bytes"
    );
    assert!(!m["truncated"].as_bool().unwrap());
    assert!(
        m["sha256"].as_str().map(|s| s.len()).unwrap_or(0) == 64,
        "sha256 must be recorded"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("byte-exact round-trip"),
        "evidence must carry the verdict line:\n{joined}"
    );
}

/// V2: validate -> report -> approve -> submit each append a custody
/// entry; chain length == handling steps, every entry binds the hash.
#[test]
fn custody_chain_grows() {
    let report = check_case("custody_chain_grows");
    let m = &report.metrics;
    assert_eq!(
        m["chain_len"].as_u64().unwrap(),
        5,
        "1 seal + 4 handling steps"
    );
    assert_eq!(m["steps"].as_u64().unwrap(), 4);
    let joined = report.evidence.join("\n");
    for action in ["sealed", "validated", "reported", "approved", "submitted"] {
        assert!(
            joined.contains(action),
            "evidence must show the {action} custody entry:\n{joined}"
        );
    }
}

// --- adversarial ---

/// A1: one flipped byte fails custody verification with the typed
/// tamper error; the finding is quarantined with evidence intact and
/// never advances past Candidate.
#[test]
fn tamper_detected() {
    let report = check_case("tamper_detected");
    let m = &report.metrics;
    assert!(m["tamper_detected"].as_bool().unwrap());
    assert!(m["quarantined"].as_bool().unwrap());
    assert_eq!(m["state"].as_str().unwrap(), "Candidate");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("Tampered"),
        "evidence must name the typed tamper error:\n{joined}"
    );
}

/// A2: 100 MiB of tool output is stored bounded at the 10 MiB cap with
/// the truncated marker; the hash covers the stored bytes.
#[test]
fn oversize_truncated() {
    assert_eq!(task_141::EVIDENCE_CAP_BYTES, 10 * 1024 * 1024);
    assert_eq!(task_141::OVERSIZE_INPUT_BYTES, 100 * 1024 * 1024);
    let report = check_case("oversize_truncated");
    let m = &report.metrics;
    assert_eq!(m["input_bytes"].as_u64().unwrap(), 100 * 1024 * 1024);
    assert_eq!(
        m["stored_bytes"].as_u64().unwrap(),
        task_141::EVIDENCE_CAP_BYTES as u64
    );
    assert!(m["truncated"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("10485760"),
        "evidence must name the cap in the seal marker:\n{joined}"
    );
}
