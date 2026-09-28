//! Integration tests for task-78 (model download and cache budgets).
//!
//! The seam is ABSENT: exact-token source scans over the product
//! crates find no download client — `download` 0 hits, `downloads` 1
//! hit (the safe-runtime policy in `crates/phlow-runtime/src/prompt.rs`
//! DENYING downloads), `huggingface`/`hf` 0 product hits — and every
//! `snapshot`/`blob`/`revision` hit classifies into an unrelated
//! sense (workspace/editor/run/context/experiment snapshots;
//! report/manifest/workflow revisions). Model bytes never enter
//! phlow's address space: Ollama serves models over HTTP and owns
//! the pull. The byte caps that exist guard other seams
//! (`RESPONSE_BYTES_MAX` = 2 MiB on Ollama response bodies;
//! `FILE_BYTES_MAX` = 256 KiB on workspace file reads).
//!
//! Whether phlow should gain an HF model manager (download with
//! observed-byte caps, hash-verified resume, blob cache with explicit
//! eviction, incomplete markers) is a product decision for Matt —
//! banked, not implemented on gauntlet authority.
//!
//! Four cases — 2 validation, 2 adversarial — each self-checking:
//! cases document the absence with file evidence; the driver then
//! reports the honest seam failure.

use phlow_gauntlet::tasks::task_78;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-78: cannot build Ctx: {e}"))
}

// --- validation ---

/// V1: no download client exists — the scan finds zero download
/// vocabulary outside the prompt.rs policy denial and gauntlet
/// harness vocabulary. The task-level driver then runs all four
/// cases and reports the honest seam failure: the design's pass
/// criteria need a model manager, and there is none — the
/// model-manager product decision is banked in the task-level
/// `how`, not implemented on gauntlet authority.
#[test]
fn no_download_client() {
    assert_eq!(task_78::ID, "task-78");
    assert_eq!(task_78::NAME, "model download and cache budgets");
    assert_eq!(task_78::KIND, TaskKind::Rust);
    assert_eq!(task_78::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_78::run_case("no_download_client")
        .unwrap_or_else(|e| panic!("task-78 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-download case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["download_clients"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("DENYING downloads") || joined.contains("prompt.rs"),
        "evidence must show the policy denial:\\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the model-manager product decision for Matt.
    let (where_, how) = match task_78::run(&ctx()) {
        TaskOutcome::Fail { where_, how, .. } => (where_, how),
        TaskOutcome::Pass { evidence } => panic!(
            "task-78 passed: a model manager was invented, not found\\nevidence: {evidence:?}"
        ),
    };
    assert_eq!(where_, "seam", "task-78 must fail at the seam");
    assert!(
        how.contains("product decision for Matt"),
        "the 'how' must bank the product decision: {how}"
    );
    assert!(
        how.contains("no model download/cache manager"),
        "the 'how' must name the absent manager: {how}"
    );
}

/// V2: no snapshot/blob-cache/revision machinery for models — every
/// hit classifies into an unrelated sense, and anything
/// unclassified would fail the case loudly (finding refuted).
#[test]
fn no_snapshot_revision_cache() {
    let report = task_78::run_case("no_snapshot_revision_cache")
        .unwrap_or_else(|e| panic!("task-78 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-cache case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["model_caches"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("unrelated senses"),
        "evidence must classify the hits:\\n{joined}"
    );
}

// --- adversarial ---

/// A1: no observed-byte cap exists — with no downloader there is
/// nothing to cap; the byte caps that DO exist are cited as file
/// evidence that they guard other seams (RESPONSE_BYTES_MAX on
/// Ollama bodies, FILE_BYTES_MAX on workspace reads).
#[test]
fn no_observed_byte_cap() {
    let report = task_78::run_case("no_observed_byte_cap")
        .unwrap_or_else(|e| panic!("task-78 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-cap case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["observed_byte_caps_on_downloads"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("RESPONSE_BYTES_MAX") && joined.contains("FILE_BYTES_MAX"),
        "evidence must cite the existing caps as file evidence:\\n{joined}"
    );
}

/// A2: incomplete downloads are never marked — because there are no
/// downloads, no partial files, and no resume logic; the design's
/// pass criteria are unrepresentable without a model manager, and
/// the absence documented with the scans IS the finding.
#[test]
fn incomplete_never_marked() {
    let report = task_78::run_case("incomplete_never_marked")
        .unwrap_or_else(|e| panic!("task-78 case failed to run: {e}"));
    assert!(
        report.passed,
        "incomplete case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["partial_file_markers"], 0);
}
