//! Integration tests for task-77 (quantization behavior change).
//!
//! The seam is REAL but does not meet the criteria: phlow's
//! validation pipeline (`phlow_llm::parse_chat_message` shape +
//! tool-call normalization, argument JSON parsing equivalent to the
//! private runtime path, `phlow_mcp::validate_arguments` on the
//! tool schema) rejects malformed tool calls with named violations —
//! the FP16 fixture (4/4 schema-valid) passes, INT8 (3/4) degrades,
//! and INT4 (1/4) degrades further. But there is no model-weight
//! quantization selection in phlow: models are served externally by
//! Ollama, and `phlow_inference::kv_policy` is KV-cache policy, not
//! model-loading precision control — the design's precision ladder
//! (FP16 -> INT8 -> INT4, degradation curves, INT8/INT4 gates) has no
//! implementation.
//!
//! Whether phlow should measure/verify Ollama-served precision (and
//! record it in run records) is a product decision for Matt —
//! banked, not implemented on gauntlet authority.
//!
//! Four cases — 2 validation, 2 adversarial — each self-checking:
//! cases drive the real validation seam with simulated fixture
//! rates; the driver then reports the honest seam failure.

use phlow_gauntlet::tasks::task_77;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-77: cannot build Ctx: {e}"))
}

// --- validation ---

/// V1: the FP16 default — clean tool-call JSON is fully valid through
/// the real pipeline (shape, ids, argument parsing, schema). This is
/// the mechanism baseline the degraded levels are measured against.
/// The task-level driver then runs all four cases and reports the
/// honest seam failure: the pipeline validates SHAPES, not
/// precisions — the measure/verify-Ollama-precision product decision
/// is banked in the task-level `how`, not implemented on gauntlet
/// authority.
#[test]
fn fp16_baseline_tool_calls_valid() {
    assert_eq!(task_77::ID, "task-77");
    assert_eq!(task_77::NAME, "quantization behavior change");
    assert_eq!(task_77::KIND, TaskKind::Rust);
    assert_eq!(task_77::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_77::run_case("fp16_baseline_tool_calls_valid")
        .unwrap_or_else(|e| panic!("task-77 case failed to run: {e}"));
    assert!(
        report.passed,
        "fp16 case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["level"], "FP16");
    assert_eq!(report.metrics["valid_rate"], 1.0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("4/4"),
        "evidence must show the full pass rate:\\n{joined}"
    );
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the precision product decision for Matt.
    let (where_, how) = match task_77::run(&ctx()) {
        TaskOutcome::Fail { where_, how, .. } => (where_, how),
        TaskOutcome::Pass { evidence } => panic!(
            "task-77 passed: a precision ladder was invented, not found\\nevidence: {evidence:?}"
        ),
    };
    assert_eq!(where_, "seam", "task-77 must fail at the seam");
    assert!(
        how.contains("product decision for Matt"),
        "the 'how' must bank the product decision: {how}"
    );
    assert!(
        how.contains("no model loading / quantization selection seam"),
        "the 'how' must name the missing ladder: {how}"
    );
}

/// V2: the design's core demand — per-quantization quality MEASURED
/// on fixtures, not assumed. The driver measures the schema-valid
/// rate per simulated level and requires the degradation ordering
/// FP16 > INT8 > INT4 (4/4, 3/4, 1/4) — measured by the harness,
/// never performed by phlow.
#[test]
fn degradation_measured_per_level() {
    let report = task_77::run_case("degradation_measured_per_level")
        .unwrap_or_else(|e| panic!("task-77 case failed to run: {e}"));
    assert!(
        report.passed,
        "degradation case must hold: {}",
        report.failures.join("; ")
    );
    let fp16 = report.metrics["fp16"].as_f64().unwrap();
    let int8 = report.metrics["int8"].as_f64().unwrap();
    let int4 = report.metrics["int4"].as_f64().unwrap();
    assert!(
        fp16 > int8 && int8 > int4,
        "degradation ordering must hold: FP16={fp16}, INT8={int8}, INT4={int4}"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("measured schema-valid rates"),
        "evidence must show the measured rates:\\n{joined}"
    );
}

// --- adversarial ---

/// A1: INT4-style almost-valid tool calls — dropped quotes, renamed
/// args, non-object arguments, duplicate ids, non-object function.
/// Every one is rejected with the schema/shape violation NAMED
/// (six rejections); none is executed.
#[test]
fn almost_valid_tool_calls_rejected() {
    let report = task_77::run_case("almost_valid_tool_calls_rejected")
        .unwrap_or_else(|e| panic!("task-77 case failed to run: {e}"));
    assert!(
        report.passed,
        "adversarial case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["adversarial_rejected"], 6);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("rejected at"),
        "evidence must name the rejection stages:\\n{joined}"
    );
}

/// A2: the quantization level is not recorded — the only
/// quantization in phlow is `phlow_inference::kv_policy`
/// (KV-cache precision, bytes/token footprint), never consulted at
/// model-load time because there is no model-load time; the run
/// report has no quantization/precision field.
#[test]
fn quantization_level_not_recorded() {
    let report = task_77::run_case("quantization_level_not_recorded")
        .unwrap_or_else(|e| panic!("task-77 case failed to run: {e}"));
    assert!(
        report.passed,
        "not-recorded case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["quantization_selection_exists"], false);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("KV-cache"),
        "evidence must place kv_policy correctly:\\n{joined}"
    );
}
