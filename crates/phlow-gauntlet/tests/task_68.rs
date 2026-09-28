//! Integration tests for task-68 (metric cardinality bound, Rust,
//! adversarial).
//!
//! There is NO metrics/telemetry registry in the phlow workspace: a
//! runtime vocabulary scan over every `crates/*/src/**/*.rs` finds zero
//! registry/label/series tokens, no crate depends on a metrics library,
//! and the one counting mechanism that exists
//! (`report_mut::{counter,add,set,push}` — a private module in
//! phlow-runtime) is a literal-keyed JSON report object with no label
//! sets and no attacker-reachable key construction. The design's
//! adversarial weapons (a per-run-id label; label values from untrusted
//! tool output) have no target: there is no label-registration API to
//! attack and no label path for untrusted output to travel. The driver
//! is an audit-only driver and reports the honest
//! `Fail { where: "seam" }`. These tests: 2 validation + 2 adversarial —
//! three drive individual cases through `run_case` with the same metrics
//! JSON a harness would collect, and the last one folds in the
//! task-level verdict.
//!
//! Product decision (banked for Matt): whether phlow should gain a
//! labeled metrics/telemetry registry at all; if it does, the hard
//! series cap, the explicit rejection signal (the design's
//! `CardinalityExceeded`), and the label-value sanitization policy for
//! untrusted tool output.

use phlow_gauntlet::tasks::task_68;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_68::CaseReport {
    let report = task_68::run_case(case)
        .unwrap_or_else(|e| panic!("task-68: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-68 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-68-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-68 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-68 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; the registry-vocabulary scan
/// over the real workspace finds zero hits and no crate depends on a
/// metrics library — no labeled registry exists anywhere.
#[test]
fn no_metrics_registry_in_sources() {
    assert_eq!(task_68::ID, "task-68");
    assert_eq!(task_68::NAME, "metric cardinality bound");
    assert_eq!(task_68::KIND, TaskKind::Rust);
    assert_eq!(
        task_68::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("no_metrics_registry_in_sources");
    assert_eq!(
        report.metrics["registry_hits"],
        serde_json::json!(0),
        "no registry-mechanism vocabulary may exist in sources"
    );
    assert_eq!(
        report.metrics["metrics_deps"],
        serde_json::json!(0),
        "no crate may depend on a metrics library"
    );
}

/// V: the one counting mechanism (`report_mut::` in phlow-runtime) is
/// literal-keyed — zero dynamic keys — so there are no label sets to
/// bound and no series to cap. Classified adjacent, not the seam.
#[test]
fn plain_counters_have_no_labels() {
    let report = run_case("plain_counters_have_no_labels");
    assert_eq!(
        report.metrics["dynamic_keys"],
        serde_json::json!(0),
        "no attacker-reachable key construction may exist"
    );
    assert!(
        report.metrics["call_sites"].as_u64().unwrap_or(0) > 0,
        "the probe must have found the counter call sites"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("adjacent, not the seam"),
        "evidence must classify the counters:\n{evidence}"
    );
}

// --- adversarial ---

/// A: the per-run-id label weapon has no target — the label-API
/// vocabulary scan finds zero registration surface, so there is
/// nothing to reject or hash with an explicit rejection signal.
#[test]
fn per_run_label_attack_has_no_target() {
    let report = run_case("per_run_label_attack_has_no_target");
    assert_eq!(
        report.metrics["label_api_hits"],
        serde_json::json!(0),
        "no label-registration API may exist"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("no target"),
        "evidence must state the weapon has no target:\n{evidence}"
    );
}

/// A: untrusted tool output cannot become a label — there is no label
/// path, only static report keys — and the task-level verdict is the
/// honest `Fail { where: "seam" }`: no metrics/telemetry registry
/// exists in any phlow crate.
#[test]
fn untrusted_output_cannot_become_a_label_and_task_fails_at_seam() {
    let report = run_case("untrusted_output_cannot_become_a_label");
    assert_eq!(
        report.metrics["label_paths"],
        serde_json::json!(0),
        "no label path may exist for untrusted output"
    );
    match task_68::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-68 must fail at the absent seam");
            assert!(
                how.contains("no metrics/telemetry registry"),
                "the 'how' must name the missing registry: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-68 passed: a metrics registry was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
