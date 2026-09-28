//! Integration tests for task-81 (provider auth failure modes).
//!
//! The seam is ABSENT: the only provider client is OllamaBackend
//! (crates/phlow-llm), and Ollama needs no auth — exact-token scans
//! for `authorization`, `api_key`, `bearer`, `401`, `403` over
//! phlow-llm/src find zero hits, and OllamaConfig has no key field.
//! `LlmError` has no auth variant, so a 401/403 would arrive as the
//! untyped `Transport(String)`. 401s are never retried (the backend
//! makes exactly one post_chat call — no retry loop exists), but
//! with no typed `auth_failed` error to carry the rule, expired vs
//! revoked vs malformed vs wrong-project vs insufficient-scope are
//! indistinguishable.
//!
//! Whether phlow should gain API-key auth with typed auth-failure
//! classification is a product decision for Matt — banked, not
//! implemented on gauntlet authority.
//!
//! Four cases — 2 validation, 2 adversarial — each self-checking:
//! cases probe the seam and record measured mechanism evidence; the
//! driver then reports the honest seam failure.

use phlow_gauntlet::tasks::task_81;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-81: cannot build Ctx: {e}"))
}

// --- validation ---

/// V1: there is no auth surface to classify — zero auth-vocabulary
/// hits in phlow-llm/src and no key field on OllamaConfig. The
/// task-level driver then runs all four cases and reports the honest
/// seam failure: the auth-failure classification seam has no
/// implementation — the API-key-auth product decision is banked in
/// the task-level `how`, not implemented on gauntlet authority.
#[test]
fn no_auth_surface() {
    assert_eq!(task_81::ID, "task-81");
    assert_eq!(task_81::NAME, "provider auth failure modes");
    assert_eq!(task_81::KIND, TaskKind::Rust);
    assert_eq!(task_81::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_81::run_case("no_auth_surface")
        .unwrap_or_else(|e| panic!("task-81 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-auth-surface case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["auth_vocabulary_hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no key field"),
        "evidence must show OllamaConfig carries no key:\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the API-key-auth product decision for Matt.
    let (where_, how) = match task_81::run(&ctx()) {
        TaskOutcome::Fail { where_, how, .. } => (where_, how),
        TaskOutcome::Pass { evidence } => panic!(
            "task-81 passed: an auth classifier was invented, not found\nevidence: {evidence:?}"
        ),
    };
    assert_eq!(where_, "seam", "task-81 must fail at the seam");
    assert!(
        how.contains("product decision for Matt"),
        "the 'how' must bank the product decision: {how}"
    );
    assert!(
        how.contains("no auth variant"),
        "the 'how' must name the missing classification: {how}"
    );
}

/// V2: auth failures are untyped — a scripted 401 arrives as the
/// untyped `Transport(String)`, and `LlmError` has no auth variant
/// to distinguish expired vs revoked vs malformed vs wrong-project.
#[test]
fn auth_failures_untyped() {
    let report = task_81::run_case("auth_failures_untyped")
        .unwrap_or_else(|e| panic!("task-81 case failed to run: {e}"));
    assert!(
        report.passed,
        "untyped-auth case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["typed_auth_variant"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("UNTYPED Transport"),
        "evidence must show the 401 arrived untyped:\n{joined}"
    );
}

// --- adversarial ---

/// A1: a 401 on every call fails fast with zero retries — measured:
/// a scripted 401 followed by a queued success leaves chat returning
/// the 401 error (one attempt, the success reply unconsumed). The
/// design's no-retry-on-401 half holds vacuously (no retry loop
/// exists), but the error is untyped — no `auth_failed` class.
#[test]
fn auth_failure_never_retried() {
    let report = task_81::run_case("auth_failure_never_retried")
        .unwrap_or_else(|e| panic!("task-81 case failed to run: {e}"));
    assert!(
        report.passed,
        "never-retried case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["attempts"], 1);
    assert_eq!(report.metrics["typed_auth_failed"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("exactly one attempt"),
        "evidence must show the attempt count:\n{joined}"
    );
}

/// A2: a 403 with an insufficient-scope message is indistinguishable
/// from any other transport failure — the scope signal lives only in
/// free-text detail, so no distinct actionable message (widen scope
/// vs rotate key) can be rendered.
#[test]
fn scope_error_untyped() {
    let report = task_81::run_case("scope_error_untyped")
        .unwrap_or_else(|e| panic!("task-81 case failed to run: {e}"));
    assert!(
        report.passed,
        "scope case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["scope_variant"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("free-text detail"),
        "evidence must show the scope signal is untyped:\n{joined}"
    );
}
