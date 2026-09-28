//! Integration tests for task-43 (secrets in error messages, Rust).
//!
//! There is NO structural secret-type seam on phlow's error plane: a
//! runtime vocabulary scan over every `crates/*/src/**/*.rs` finds
//! zero structural-secret tokens (no secret-typed wrappers, no masked
//! markers, no field-level debug skipping, no memory-clearing
//! wrappers). The sole redaction mechanism in the tree is
//! `strip_source_echo` (phlow-config) — string-scrubbing of TOML
//! parse-error source echoes at one site, which the design explicitly
//! excludes ("the redaction is structural, not string-matching").
//! There are additionally no secret-bearing fields to mask: URL
//! validation rejects credentials and the operator registry holds
//! public keys only. The design's adversarial scenarios (a tool call
//! failing with a secret in its arguments; `Debug` formatting of a
//! config struct) have no target. The driver is an audit-only driver
//! and reports the honest `Fail { where: "seam" }`. These tests:
//! 2 validation + 2 adversarial — three drive individual cases through
//! `run_case` with the same metrics JSON a harness would collect, and
//! the last one folds in the task-level verdict.
//!
//! Product decision (banked for Matt): whether phlow should introduce
//! structural secret types with masked `Display`/`Debug`.

use phlow_gauntlet::tasks::task_43;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_43::CaseReport {
    let report = task_43::run_case(case)
        .unwrap_or_else(|e| panic!("task-43: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-43 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-43-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-43 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-43 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; the structural-secret
/// vocabulary scan over the real workspace finds zero hits — no
/// secret-typed wrappers, masked markers, field-level debug skipping,
/// or memory-clearing wrappers in any phlow source.
#[test]
fn no_structural_secret_types_in_sources() {
    assert_eq!(task_43::ID, "task-43");
    assert_eq!(task_43::NAME, "secrets in error messages");
    assert_eq!(task_43::KIND, TaskKind::Rust);
    assert_eq!(
        task_43::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("no_structural_secret_types_in_sources");
    assert_eq!(
        report.metrics["wrapper_hits"],
        serde_json::json!(0),
        "no structural-secret vocabulary may exist in sources"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("hits: 0"),
        "evidence must show the hit count:\n{evidence}"
    );
}

/// V: the sole redaction mechanism is documented as non-structural —
/// `strip_source_echo` scrubs TOML parse-error echoes at one site;
/// `structural_types` is 0, so the design's structural criterion is
/// unmet and the probe does not claim a pass.
#[test]
fn sole_redaction_is_string_scrub() {
    let report = run_case("sole_redaction_is_string_scrub");
    assert_eq!(
        report.metrics["structural_types"],
        serde_json::json!(0),
        "no structural secret types may exist"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("strip_source_echo"),
        "evidence must name the one string-scrub mechanism:\n{evidence}"
    );
    assert!(
        evidence.contains("not a secret-typed wrapper"),
        "evidence must classify the scrubber as non-structural:\n{evidence}"
    );
}

// --- adversarial ---

/// A: the failing-tool-call scenario has no target —
/// `secret_typed_fields` is 0. A secret planted in a plain `String`
/// argument would render verbatim: no masking layer exists to
/// intercept it.
#[test]
fn failing_tool_call_has_no_masked_args() {
    let report = run_case("failing_tool_call_has_no_masked_args");
    assert_eq!(
        report.metrics["secret_typed_fields"],
        serde_json::json!(0),
        "no secret-typed fields may exist to mask"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("render verbatim"),
        "evidence must state the unmasked rendering consequence:\n{evidence}"
    );
}

/// A: config `Debug` derives have no field-level skips — and nothing
/// secret to skip (URL validation rejects credentials; the operator
/// registry holds public keys only). The task-level verdict is the
/// honest `Fail { where: "seam" }`.
#[test]
fn config_debug_derives_have_no_skips_and_task_fails_at_seam() {
    let report = run_case("config_debug_derives_have_no_skips");
    assert_eq!(
        report.metrics["field_level_skips"],
        serde_json::json!(0),
        "no field-level debug skips may exist"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("PUBLIC keys only"),
        "evidence must show the registry holds no secret material:\n{evidence}"
    );
    match task_43::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-43 must fail at the absent seam");
            assert!(
                how.contains("no structural secret types"),
                "the 'how' must name the missing wrapper types: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-43 passed: structural secret types were invented, not found\nevidence: {evidence:?}"
        ),
    }
}
