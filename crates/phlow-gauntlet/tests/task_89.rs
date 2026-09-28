//! Integration tests for task-89 (protocol version skew).
//!
//! The seam is REAL and half-capable: `phlow-mcp`'s
//! `McpServer::initialize` negotiates KNOWN versions correctly
//! (matching and skew), but unknown versions — garbage AND future
//! alike — silently negotiate to the NEWEST supported version with
//! no typed error, and a mid-session version change is rejected
//! without invalidating the session. Each test drives the real
//! `McpServer<FakeRuntime>` and asserts the honest `fail` at
//! `"seam"`: 2 validation, 2 adversarial.
//!
//! Product decisions banked for Matt (phlow-owned, not implemented
//! on gauntlet authority): whether unknown versions should fail
//! closed with a typed `version_negotiation_failed`, and whether a
//! mid-session re-initialize should invalidate the session. Diver's
//! client side (sends `protocolVersion = '2024-11-05'`, discards the
//! negotiated version) is diver-owned and flagged.

use phlow_gauntlet::tasks::task_89;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-89: cannot build Ctx: {e}"))
}

/// Unwrap the expected `fail` at `"seam"`, or panic with the details.
fn fail_at_seam(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-89 passed: version-skew discipline was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: matching versions negotiate to the offered version — the
/// negotiation works for known versions. The task-level driver then
/// runs all four cases and reports the honest seam failure: the
/// skew-discipline product decisions are banked in the task-level
/// `how`, not implemented on gauntlet authority.
#[test]
fn matching_versions_negotiate() {
    assert_eq!(task_89::ID, "task-89");
    assert_eq!(task_89::NAME, "protocol version skew");
    assert_eq!(task_89::KIND, TaskKind::Rust);
    assert_eq!(task_89::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_89::run_case("matching_versions_negotiate")
        .unwrap_or_else(|e| panic!("task-89 case failed to run: {e}"));
    assert!(
        report.passed,
        "matching-versions case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["negotiated"], "2025-11-25");
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the product decisions for Matt.
    let (where_, how, _) = fail_at_seam(task_89::run(&ctx()));
    assert_eq!(where_, "seam", "task-89 must fail at the seam");
    assert!(
        how.contains("silently negotiate"),
        "the 'how' must name the silent upgrade: {how}"
    );
    assert!(
        how.contains("Product decisions banked for Matt"),
        "the 'how' must bank the product decisions: {how}"
    );
}

/// V2: skew (client N-1) negotiates the greatest mutually supported
/// version — the supported-version path is correct.
#[test]
fn skew_negotiates_older() {
    let report = task_89::run_case("skew_negotiates_older")
        .unwrap_or_else(|e| panic!("task-89 case failed to run: {e}"));
    assert!(
        report.passed,
        "skew case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["negotiated"], "2025-06-18");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("the supported-version path is correct"),
        "evidence must confirm the correct path:\n{joined}"
    );
}

// --- adversarial ---

/// A1: garbage ("banana") and a future version ("2999-99-99") both
/// silently negotiate to the newest supported version — no typed
/// error, no fail-closed. The server cannot distinguish a future
/// version from garbage, and a client implementing only an older
/// version is told the server speaks the newest.
#[test]
fn garbage_and_future_silently_upgrade() {
    let report = task_89::run_case("garbage_and_future_silently_upgrade")
        .unwrap_or_else(|e| panic!("task-89 case failed to run: {e}"));
    assert!(
        report.passed,
        "garbage/future case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["banana"], "2025-11-25");
    assert_eq!(report.metrics["2999-99-99"], "2025-11-25");
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("typed `version_negotiation_failed` does not exist"),
        "evidence must state the missing typed error:\n{joined}"
    );
}

/// A2: a mid-session version change does not invalidate the session.
/// After initialize("2025-06-18") + notifications/initialized, a
/// second initialize("2025-11-25") is rejected with -32600
/// ("Already initialized") but the session continues on the old
/// version — ping still succeeds.
#[test]
fn mid_session_change_not_invalidated() {
    let report = task_89::run_case("mid_session_change_not_invalidated")
        .unwrap_or_else(|e| panic!("task-89 case failed to run: {e}"));
    assert!(
        report.passed,
        "mid-session-change case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["second_initialize_code"], -32600);
    assert_eq!(report.metrics["session_invalidated"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("the session survives on the old version"),
        "evidence must show the surviving session:\n{joined}"
    );
}
