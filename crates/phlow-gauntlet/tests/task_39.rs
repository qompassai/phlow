//! Integration tests for task-39 (recursive payload bomb, Rust).
//!
//! The seam is PARTIALLY present and reported honestly: `phlow_json`
//! bounds depth (explicit stack, `JSON_DEPTH_MAX = 64`, typed
//! `JsonError::DepthExceeded`; serde_json's own recursion limit at
//! parse time) and total size (1 MiB, typed `JsonError::InputTooLarge`),
//! but the design CONJUNCTIVELY demands an expansion-ratio bound with
//! its own typed error — and no such bound exists (no constant, no
//! `JsonError` variant; a source scan of the embedded phlow-json
//! sources confirms it). The billion-laughs attack itself has no seam
//! (no recursive-expansion decoder in phlow's parse paths: no entity
//! expansion, no archive extraction), so the missing bound is a
//! defense-in-depth product decision, not a demonstrated vulnerability —
//! but the conjunctive criteria are unmet, hence `Fail { where: "seam" }`.
//! These tests: 2 validation + 2 adversarial — three drive individual
//! cases through `run_case` with the same metrics JSON a harness would
//! collect, and the last one folds in the task-level verdict.
//!
//! Product decision (banked for Matt): whether phlow_json should grow
//! an expansion-ratio bound (typed `JsonError` variant) as
//! defense-in-depth.

use phlow_gauntlet::tasks::task_39;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_39::CaseReport {
    let report = task_39::run_case(case)
        .unwrap_or_else(|e| panic!("task-39: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-39 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-39-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-39 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-39 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; a nested payload parses (the
/// happy path is real) and the parsed node count cannot self-amplify.
#[test]
fn nested_payload_parses() {
    assert_eq!(task_39::ID, "task-39");
    assert_eq!(task_39::NAME, "recursive payload bomb");
    assert_eq!(task_39::KIND, TaskKind::Rust);
    assert_eq!(
        task_39::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("nested_payload_parses");
    assert_eq!(
        report.metrics["parsed"],
        serde_json::json!(true),
        "the nested payload must parse"
    );
}

/// V: a 10_000-deep payload is REJECTED with a typed depth error — the
/// explicit `JSON_DEPTH_MAX` (64) bound is real and enforced after parse
/// with an explicit stack (no recursion over attacker-controlled depth),
/// backed by serde_json's own recursion limit at parse time.
#[test]
fn deep_nesting_rejected_with_typed_error() {
    let report = run_case("deep_nesting_rejected_with_typed_error");
    assert_eq!(
        report.metrics["explicit_bound"],
        serde_json::json!(64),
        "the explicit depth bound must be 64"
    );
    assert_eq!(
        report.metrics["rejected_typed"],
        serde_json::json!(true),
        "the deep input must be rejected with a typed error"
    );
}

// --- adversarial ---

/// A: the expansion-RATIO bound is absent — the metric
/// `ratio_bound_in_source` is false, measured against a large flat
/// payload (parsed nodes <= input bytes: no self-amplification, because
/// JSON cannot expand beyond O(input)). No recursive-expansion decoder
/// exists in phlow's parse paths, so the billion-laughs attack has no
/// seam — but the design's conjunctive criterion is unmet.
#[test]
fn expansion_ratio_bound_absent() {
    let report = run_case("expansion_ratio_bound_absent");
    assert_eq!(
        report.metrics["ratio_bound_in_source"],
        serde_json::json!(false),
        "no expansion-ratio bound may exist in the source"
    );
    let input = report.metrics["input_bytes"].as_u64().unwrap_or(0);
    let nodes = report.metrics["parsed_nodes"].as_u64().unwrap_or(1);
    assert!(
        nodes <= input,
        "a flat payload must not self-amplify: nodes={nodes} bytes={input}"
    );
}

/// A: the flat 1 MiB cap is the only total-size bound — an oversize
/// payload is rejected with a typed error; and the task-level verdict
/// is the honest `Fail { where: "seam" }` because the design
/// conjunctively demands a ratio bound too.
#[test]
fn flat_cap_is_the_only_total_bound_and_task_fails_at_seam() {
    let report = run_case("flat_cap_is_the_only_total_bound");
    assert_eq!(
        report.metrics["input_bytes_max"],
        serde_json::json!(1_048_576),
        "the flat cap must be 1 MiB"
    );
    assert_eq!(
        report.metrics["rejected_typed"],
        serde_json::json!(true),
        "the oversize input must be rejected with a typed error"
    );
    match task_39::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-39 must fail at the absent seam");
            assert!(
                how.contains("expansion-ratio bound"),
                "the 'how' must name the missing conjunct: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-39 passed: an expansion-ratio bound was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
