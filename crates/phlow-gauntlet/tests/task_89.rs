//! Integration tests for task-89 (protocol version skew).
//!
//! The seam moved: the hand-rolled dual-era server negotiated
//! versions in `McpServer::initialize`, and did it half-right —
//! known versions negotiated, unknown versions (garbage AND
//! future) silently upgraded to the newest, and a mid-session
//! re-initialize was rejected without invalidating the session.
//! This task honestly failed at `"seam"` on that evidence.
//!
//! The fleet standardization (D-113/D-200) and phlow's rmcp
//! migration (Stage C2) deleted the negotiation entirely: the
//! server is modern-only, and the seam is the era guard's
//! admission function `phlow_mcp::classify`. Every non-modern
//! version contact now fails closed with the typed `-32022`
//! refusal; modern frames are admitted statelessly; mid-session
//! version changes are impossible by construction. Each test
//! drives the real guard and asserts the honest `pass`:
//! 2 validation, 2 adversarial.

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

// --- validation ---

/// V1: `initialize` at legacy versions is refused with the typed
/// -32022 error naming the requested version and the one
/// supported version. The task-level driver runs all four cases
/// and passes: the skew discipline now holds at the real seam.
#[test]
fn legacy_initialize_refused_typed() {
    assert_eq!(task_89::ID, "task-89");
    assert_eq!(task_89::NAME, "protocol version skew");
    assert_eq!(task_89::KIND, TaskKind::Rust);
    assert_eq!(task_89::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_89::run_case("legacy_initialize_refused_typed")
        .unwrap_or_else(|e| panic!("task-89 case failed to run: {e}"));
    assert!(
        report.passed,
        "legacy-initialize case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["2025-06-18"], -32022);
    assert_eq!(report.metrics["2025-11-25"], -32022);
    // Task-level: the driver passes (the gap it documented is
    // closed), and the evidence names the closure.
    match task_89::run(&ctx()) {
        TaskOutcome::Pass { evidence } => {
            let joined = evidence.join("\n");
            assert!(
                joined.contains("closed by the rmcp migration"),
                "evidence must name the closure:\n{joined}"
            );
        }
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-89 must pass after the migration: fail at {where_}: {how}\n{evidence:?}"),
    }
}

/// V2: modern frames (discover, list, call) are admitted
/// verbatim — stateless, no handshake.
#[test]
fn modern_discover_admitted() {
    let report = task_89::run_case("modern_discover_admitted")
        .unwrap_or_else(|e| panic!("task-89 case failed to run: {e}"));
    assert!(
        report.passed,
        "modern-admission case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["forwarded"], 3);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("admitted statelessly"),
        "evidence must confirm stateless admission:\n{joined}"
    );
}

// --- adversarial ---

/// A1: garbage ("banana") and a future version ("2999-99-99")
/// are refused identically — same typed code, same shape — with
/// the requested version echoed so the record distinguishes
/// them. Neither unlocks anything.
#[test]
fn garbage_and_future_refused_identically() {
    let report = task_89::run_case("garbage_and_future_refused_identically")
        .unwrap_or_else(|e| panic!("task-89 case failed to run: {e}"));
    assert!(
        report.passed,
        "garbage/future case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["banana"], -32022);
    assert_eq!(report.metrics["2999-99-99"], -32022);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("version_negotiation_failed exists"),
        "evidence must state the typed refusal exists:\n{joined}"
    );
}

/// A2: an `initialize` after modern traffic is refused exactly
/// like one arriving first, and the modern traffic around it is
/// unaffected — there is no session to invalidate.
#[test]
fn mid_session_initialize_still_refused() {
    let report = task_89::run_case("mid_session_initialize_still_refused")
        .unwrap_or_else(|e| panic!("task-89 case failed to run: {e}"));
    assert!(
        report.passed,
        "mid-session case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["initialize_code"], -32022);
    assert_eq!(report.metrics["session_invalidated"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("met by construction"),
        "evidence must show the by-construction argument:\n{joined}"
    );
}
