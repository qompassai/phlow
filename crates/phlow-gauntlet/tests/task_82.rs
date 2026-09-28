//! Integration tests for task-82 (provider rate-limit protocol compliance).
//!
//! The seam is ABSENT: phlow-llm speaks no rate-limit protocol —
//! exact-token scans for `429`, `retry-after`, `retry_after`, and
//! `quota` over phlow-llm/src find zero hits; `LlmError` has no
//! rate-limit variant; `LlmTransport::post_chat` takes
//! (base_url, payload, timeout) with no header slot, so Retry-After
//! could not reach the backend even if a transport read it.
//! OllamaBackend::chat makes exactly one post_chat call (no retry
//! loop), so 429/503/401 are not distinct code paths, clamped waits
//! and bounded give-up are unrepresentable, and per-provider quota
//! state does not exist.
//!
//! Whether phlow-llm should speak the 429/Retry-After protocol
//! (honored-and-clamped waits, bounded give-up, per-provider quota)
//! is a product decision for Matt — banked, not implemented on
//! gauntlet authority.
//!
//! Four cases — 2 validation, 2 adversarial — each self-checking:
//! cases probe the seam and record measured mechanism evidence; the
//! driver then reports the honest seam failure.

use phlow_gauntlet::tasks::task_82;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-82: cannot build Ctx: {e}"))
}

// --- validation ---

/// V1: 429 has no classification — zero `429` hits in phlow-llm/src,
/// and a scripted 429 arrives as the untyped `Transport(String)`.
/// The task-level driver then runs all four cases and reports the
/// honest seam failure: the rate-limit protocol has no
/// implementation — the 429-protocol product decision is banked in
/// the task-level `how`, not implemented on gauntlet authority.
#[test]
fn no_429_classification() {
    assert_eq!(task_82::ID, "task-82");
    assert_eq!(task_82::NAME, "provider rate-limit protocol compliance");
    assert_eq!(task_82::KIND, TaskKind::Rust);
    assert_eq!(task_82::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_82::run_case("no_429_classification")
        .unwrap_or_else(|e| panic!("task-82 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-429 case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["status_429_hits"], 0);
    assert_eq!(report.metrics["typed_429_variant"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("untyped Transport"),
        "evidence must show the 429 arrived untyped:\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the rate-limit-protocol product decision for Matt.
    let (where_, how) = match task_82::run(&ctx()) {
        TaskOutcome::Fail { where_, how, .. } => (where_, how),
        TaskOutcome::Pass { evidence } => panic!(
            "task-82 passed: a rate-limit protocol was invented, not found\nevidence: {evidence:?}"
        ),
    };
    assert_eq!(where_, "seam", "task-82 must fail at the seam");
    assert!(
        how.contains("product decision for Matt"),
        "the 'how' must bank the product decision: {how}"
    );
    assert!(
        how.contains("no rate-limit variant"),
        "the 'how' must name the missing classification: {how}"
    );
}

/// V2: `Retry-After` is never parsed — zero hits, and the trait
/// signature has no header slot: the header has no wire-to-decision
/// path even in principle.
#[test]
fn no_retry_after_parsing() {
    let report = task_82::run_case("no_retry_after_parsing")
        .unwrap_or_else(|e| panic!("task-82 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-retry-after case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["retry_after_hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no header parameter"),
        "evidence must show the trait has no header slot:\n{joined}"
    );
}

// --- adversarial ---

/// A1: there is no bounded retry — because there is no retry at all.
/// Five calls against an always-429 fake consume exactly five
/// scripted replies (one attempt per call, zero internal retries):
/// the design's clamped waits and bounded give-up
/// (`rate_limit_exhausted`) are unrepresentable — the retry loop
/// they would live in does not exist.
#[test]
fn no_bounded_retry() {
    let report = task_82::run_case("no_bounded_retry")
        .unwrap_or_else(|e| panic!("task-82 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-bounded-retry case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["calls"], 5);
    assert_eq!(report.metrics["attempts_per_call"], 1);
    assert_eq!(report.metrics["retry_loop"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no retry loop"),
        "evidence must name the missing loop:\n{joined}"
    );
}

/// A2: per-provider quota state does not exist — zero `quota` hits,
/// and the backend holds (cfg, base_url, transport) only: no
/// counters to leak across providers because there are no counters.
#[test]
fn no_per_provider_quota_state() {
    let report = task_82::run_case("no_per_provider_quota_state")
        .unwrap_or_else(|e| panic!("task-82 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-quota case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["quota_hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no counters"),
        "evidence must show the backend holds no state:\n{joined}"
    );
}
