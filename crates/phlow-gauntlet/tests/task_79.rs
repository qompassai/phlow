//! Integration tests for task-79 (offline fallback).
//!
//! The seam is REAL but does not meet the criteria: phlow's model
//! resolution is a config string passed verbatim to Ollama
//! (`FlowConfig::model_for` -> `build_chat_payload`'s `model` field,
//! unchanged) — there is no model cache, no revision pinning
//! (`set_model` accepts any non-empty string; a `@sha256:` suffix is
//! unverified characters), no content-hash verification, no offline
//! mode, and no `offline: true` run-record label. Offline detection
//! is boolean (`OllamaBackend::is_available()` false on any transport
//! failure) and `chat` fails closed with the untyped
//! `LlmError::Transport` — no fallback, no silent substitution, but
//! not the design's typed `model_unavailable_offline`, and it names
//! no revision because there is no revision to name. A provider-side
//! tag move (same tag, different bytes) is undetectable by phlow.
//!
//! Whether phlow should pin model revisions (content-hash-verified
//! local cache, `offline: true` labeling, typed
//! `model_unavailable_offline`) is a product decision for Matt —
//! banked, not implemented on gauntlet authority.
//!
//! Four cases — 2 validation, 2 adversarial — each self-checking:
//! cases drive the real config/transport seam; the driver then
//! reports the honest seam failure.

use phlow_gauntlet::tasks::task_79;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-79: cannot build Ctx: {e}"))
}

// --- validation ---

/// V1: the design's default — hub reachable. The configured model
/// resolves through `model_for`, the fake `/api/tags` lists it,
/// `is_available()` is true, and the real payload carries the model
/// name verbatim. The task-level driver then runs all four cases
/// and reports the honest seam failure: offline means
/// transport-error, not a mode — the revision-pinning product
/// decision is banked in the task-level `how`, not implemented on
/// gauntlet authority.
#[test]
fn online_resolution() {
    assert_eq!(task_79::ID, "task-79");
    assert_eq!(task_79::NAME, "offline fallback");
    assert_eq!(task_79::KIND, TaskKind::Rust);
    assert_eq!(task_79::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_79::run_case("online_resolution")
        .unwrap_or_else(|e| panic!("task-79 case failed to run: {e}"));
    assert!(
        report.passed,
        "online case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["available"], true);
    assert_eq!(report.metrics["resolved"], "task-79-fixture");
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the revision-pinning product decision for Matt.
    let (where_, how) = match task_79::run(&ctx()) {
        TaskOutcome::Fail { where_, how, .. } => (where_, how),
        TaskOutcome::Pass { evidence } => panic!(
            "task-79 passed: revision pinning was invented, not found\\nevidence: {evidence:?}"
        ),
    };
    assert_eq!(where_, "seam", "task-79 must fail at the seam");
    assert!(
        how.contains("product decision for Matt"),
        "the 'how' must bank the product decision: {how}"
    );
    assert!(
        how.contains("no model cache"),
        "the 'how' must name the absent cache: {how}"
    );
}

/// V2: the design's "offline with the pinned revision cached" is
/// unrepresentable — model identity is an opaque string,
/// `set_model` validates only non-emptiness (a fake `@sha256:`
/// suffix is accepted as mere characters), and there is no cache
/// dir, no revision field, no offline label, no content-hash
/// verification anywhere in the model path.
#[test]
fn offline_pinned_revision_absent() {
    let report = task_79::run_case("offline_pinned_revision_absent")
        .unwrap_or_else(|e| panic!("task-79 case failed to run: {e}"));
    assert!(
        report.passed,
        "pinned-revision case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["revision_pinning_exists"], false);
    assert_eq!(report.metrics["offline_label_exists"], false);
}

// --- adversarial ---

/// A1: offline WITHOUT the pinned model — the fake hub refuses,
/// `is_available()` is false, and `chat` fails closed with the
/// UNTYPED `LlmError::Transport`: no fallback, no substitution, but
/// not `model_unavailable_offline`, and it names no revision.
#[test]
fn offline_fails_closed_untyped() {
    let report = task_79::run_case("offline_fails_closed_untyped")
        .unwrap_or_else(|e| panic!("task-79 case failed to run: {e}"));
    assert!(
        report.passed,
        "offline-fail case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["available"], false);
    assert_eq!(report.metrics["typed_offline_error"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("LlmError::Transport"),
        "evidence must show the untyped error:\\n{joined}"
    );
}

/// A2: the adversarial "helpful swap" — phlow never substitutes the
/// configured string (verbatim into the payload), but it also never
/// verifies: a provider-side tag move is undetectable.
#[test]
fn no_silent_substitution_no_verification() {
    let report = task_79::run_case("no_silent_substitution_no_verification")
        .unwrap_or_else(|e| panic!("task-79 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-substitution case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["substituted"], false);
    assert_eq!(report.metrics["verified"], false);
}
