//! Integration tests for task-144 (report generation from a
//! validated finding).
//!
//! Four driver cases — 2 validation, 2 adversarial — against scripted
//! fixtures (MOCK): a `Reportable` finding renders a section-complete
//! report with every surface redacted at write time; the rendered
//! markdown is lint-clean (and the lint is proven live against planted
//! violations); a missing field is refused with typed `MissingField`
//! and no partial report; `{{template}}` markers and fence breakouts in
//! the steps render inert inside a lengthened fence.

use phlow_gauntlet::TaskKind;
use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_144;

fn check_case(case: &str) -> CaseReport {
    let report = task_144::run_case(case)
        .unwrap_or_else(|e| panic!("task-144 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-144 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: metadata contract pins the task; all five sections render and a
/// planted secret comes out redacted, never raw.
#[test]
fn reportable_renders_complete() {
    assert_eq!(task_144::ID, "task-144");
    assert_eq!(task_144::NAME, "report-generation");
    assert_eq!(task_144::KIND, TaskKind::Rust);
    let report = check_case("reportable_renders_complete");
    let m = &report.metrics;
    assert!(m["markdown_bytes"].as_u64().unwrap() > 0);
    assert!(m["redacted"].as_bool().unwrap());
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("[REDACTED]"),
        "evidence must show the write-time redaction:\n{joined}"
    );
}

/// V2: the rendered report is lint-clean, and the lint is proven live
/// by catching planted violations.
#[test]
fn rendered_markdown_lint_clean() {
    let report = check_case("rendered_markdown_lint_clean");
    let m = &report.metrics;
    assert_eq!(m["violations"].as_u64().unwrap(), 0);
    assert!(
        m["planted_caught"].as_u64().unwrap() >= 3,
        "the lint must catch the planted violations"
    );
}

// --- adversarial ---

/// A1: a missing field refuses the whole report with the typed
/// `MissingField` — no partial document is ever emitted.
#[test]
fn missing_field_refused() {
    let report = check_case("missing_field_refused");
    let m = &report.metrics;
    let refused: Vec<&str> = m["refused_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(refused, ["impact", "summary", "steps"]);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("MissingField"),
        "evidence must name the typed refusal:\n{joined}"
    );
}

/// A2: template markers and a fence breakout in the steps render inert:
/// the literal text survives inside a lengthened fence and the lint
/// still passes.
#[test]
fn injection_rendered_inert() {
    assert_eq!(task_144::FENCE_LEN_MAX, 32);
    let report = check_case("injection_rendered_inert");
    let m = &report.metrics;
    assert_eq!(m["fence_len"].as_u64().unwrap(), 4);
    assert_eq!(m["lint_violations"].as_u64().unwrap(), 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("{{template}}"),
        "evidence must show the marker rendered literally:\n{joined}"
    );
}
