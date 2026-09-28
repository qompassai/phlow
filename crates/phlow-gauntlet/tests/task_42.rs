//! Integration tests for task-42 (audit log append-only, Rust).
//!
//! There is NO tamper-evident journal seam in the phlow workspace: a
//! runtime vocabulary scan over every `crates/*/src/**/*.rs` finds
//! zero integrity-mechanism tokens (no entry linking, no sealing keys,
//! no tamper-evidence journaling). The two grow-only structures that
//! DO exist are classified and rejected as the seam —
//! `ConsumedApprovals` (phlow-experiment) is replay protection with
//! expiring entries, `FusionReceipt.log` (phlow-tools) is an
//! in-memory `Vec<String>`. The design's adversarial weapons (byte
//! flip in an old entry, tail truncation, whole-log rewrite) have no
//! target: no writer, no verifier, no head-hash tracking. The driver
//! is an audit-only driver and reports the honest
//! `Fail { where: "seam" }`. These tests: 2 validation + 2 adversarial —
//! three drive individual cases through `run_case` with the same metrics
//! JSON a harness would collect, and the last one folds in the
//! task-level verdict.
//!
//! Product decision (banked for Matt): whether phlow should gain a
//! sealed, entry-linked journal — the approval/promotion path would be
//! the natural first consumer.

use phlow_gauntlet::tasks::task_42;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_42::CaseReport {
    let report = task_42::run_case(case)
        .unwrap_or_else(|e| panic!("task-42: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-42 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-42-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-42 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-42 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; the integrity-mechanism
/// vocabulary scan over the real workspace finds zero hits — no entry
/// linking, no sealing keys, no tamper-evidence journaling in any
/// phlow source.
#[test]
fn no_integrity_tokens_in_sources() {
    assert_eq!(task_42::ID, "task-42");
    assert_eq!(task_42::NAME, "audit log append-only");
    assert_eq!(task_42::KIND, TaskKind::Rust);
    assert_eq!(
        task_42::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("no_integrity_tokens_in_sources");
    assert_eq!(
        report.metrics["integrity_hits"],
        serde_json::json!(0),
        "no integrity-mechanism vocabulary may exist in sources"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("hits: 0"),
        "evidence must show the hit count:\n{evidence}"
    );
}

/// V: the two grow-only structures the scan DOES find are classified
/// and rejected as the seam — the approval replay store expires
/// entries (`evict_expired`, the opposite of a journal) and the fusion
/// decision log is in-memory only; zero integrity-mechanism tokens
/// appear near either structure.
#[test]
fn grow_only_stores_lack_verification() {
    let report = run_case("grow_only_stores_lack_verification");
    assert_eq!(
        report.metrics["integrity_hits_near_stores"],
        serde_json::json!(0),
        "no verification machinery may exist near the grow-only stores"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("evict_expired"),
        "evidence must name the replay store's entry expiry:\n{evidence}"
    );
    assert!(
        evidence.contains("not the seam"),
        "evidence must classify the structures as adjacent, not the seam:\n{evidence}"
    );
}

// --- adversarial ---

/// A: the byte-flip weapon has no target — `verifiers_found` is 0. No
/// tamper detection was measured because there is no journal to
/// tamper; a claimed detection would be invented, not sourced.
#[test]
fn byte_flip_has_no_verifier() {
    let report = run_case("byte_flip_has_no_verifier");
    assert_eq!(
        report.metrics["verifiers_found"],
        serde_json::json!(0),
        "the byte flip must have no verifier target"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("no journal to tamper"),
        "evidence must state the weapon has no target:\n{evidence}"
    );
}

/// A: tail truncation has no head-hash detector — and the task-level
/// verdict is the honest `Fail { where: "seam" }`: no tamper-evident
/// journal exists in any phlow crate.
#[test]
fn truncation_has_no_head_hash_and_task_fails_at_seam() {
    let report = run_case("truncation_has_no_head_hash");
    assert_eq!(
        report.metrics["head_hash_trackers"],
        serde_json::json!(0),
        "no head-hash tracking may exist"
    );
    match task_42::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-42 must fail at the absent seam");
            assert!(
                how.contains("no tamper-evident journal"),
                "the 'how' must name the missing journal: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-42 passed: a tamper-evident journal was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
