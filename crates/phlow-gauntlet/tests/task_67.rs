//! Integration tests for task-67 (audit completeness, Rust).
//!
//! There is NO audit event emitter in the phlow workspace: a runtime
//! vocabulary scan over every `crates/*/src/**/*.rs` finds zero
//! emitter-mechanism tokens, the one audit-adjacent writer
//! (`EvaluationRecord::record_event`) is a manual opt-in recorder with
//! zero call sites, and the real `promotion::Lifecycle` plus the real
//! `Evaluator` stage machine transition with no event channel
//! (`transition()` returns only the next state; `validate`/`prepare`/
//! `execute` return `Result<(), _>`). The design's mechanical
//! cross-check (event log vs transition log) has no event log to run
//! against; the adversarial "emitter errors mid-run" weapon has no
//! target. The driver is an audit-only driver and reports the honest
//! `Fail { where: "seam" }`. These tests: 2 validation + 2 adversarial —
//! three drive individual cases through `run_case` with the same metrics
//! JSON a harness would collect, and the last one folds in the
//! task-level verdict.
//!
//! Product decision (banked for Matt): whether phlow should gain an
//! audit event emitter wired into `Lifecycle::transition` and the
//! `Evaluator` stage machine (with fail-closed semantics when emission
//! fails); whether emission is synchronous at the transition site or
//! via a subscribed sink; and whether the manual
//! `EvaluationRecord::record_event` should become that emitter or stay
//! a manual recorder.

use phlow_gauntlet::tasks::task_67;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_67::CaseReport {
    let report = task_67::run_case(case)
        .unwrap_or_else(|e| panic!("task-67: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-67 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-67-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-67 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-67 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; the emitter-vocabulary scan over
/// the real workspace finds zero hits, and the manual recorder has zero
/// call sites — no transition emitter exists anywhere.
#[test]
fn no_audit_emitter_in_sources() {
    assert_eq!(task_67::ID, "task-67");
    assert_eq!(task_67::NAME, "audit completeness");
    assert_eq!(task_67::KIND, TaskKind::Rust);
    assert_eq!(
        task_67::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("no_audit_emitter_in_sources");
    assert_eq!(
        report.metrics["emitter_hits"],
        serde_json::json!(0),
        "no emitter-mechanism vocabulary may exist in sources"
    );
    assert_eq!(
        report.metrics["recorder_callers"],
        serde_json::json!(0),
        "the manual recorder must have zero production call sites"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("0 hits (want 0)"),
        "evidence must show the hit count:\n{evidence}"
    );
}

/// V: the real lifecycle walks six transitions and the real evaluator
/// walks three stage transitions; every transition returns only the
/// next state — emission is impossible by construction, not merely
/// unobserved. The `transitions: u32` counter counts, it records no
/// events (classified adjacent, not the seam).
#[test]
fn transitions_produce_no_events() {
    let report = run_case("transitions_produce_no_events");
    assert_eq!(
        report.metrics["lifecycle_transitions"],
        serde_json::json!(6),
        "the lifecycle must walk six transitions"
    );
    assert_eq!(
        report.metrics["evaluator_stages"],
        serde_json::json!(3),
        "the evaluator must walk three stage transitions"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("no emitter argument, no event in the return type"),
        "evidence must name the missing event channel:\n{evidence}"
    );
    assert!(
        evidence.contains("COUNTS stage transitions"),
        "evidence must classify the u32 counter as adjacent:\n{evidence}"
    );
}

// --- adversarial ---

/// A: six unusual paths — rejection, terminal-state rejection, expiry,
/// rollback (the compensation path), illegal transition, budget
/// exhaustion — all behave per contract (fail closed where designed)
/// and emit nothing on any of them.
#[test]
fn unusual_paths_also_silent() {
    let report = run_case("unusual_paths_also_silent");
    assert_eq!(
        report.metrics["unusual_paths"],
        serde_json::json!(6),
        "six unusual paths must be exercised"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("fails closed"),
        "evidence must show fail-closed behavior:\n{evidence}"
    );
    assert!(
        evidence.contains("RolledBack"),
        "evidence must show the compensation path:\n{evidence}"
    );
}

/// A: the design's "emitter errors mid-run" adversarial has no target —
/// phlow-experiment's public API contains no emitter/sink/audit type —
/// and the task-level verdict is the honest `Fail { where: "seam" }`:
/// no audit event emitter exists in any phlow crate.
#[test]
fn emitter_failure_has_no_target_and_task_fails_at_seam() {
    let report = run_case("emitter_failure_has_no_target");
    assert_eq!(
        report.metrics["emitter_api_types"],
        serde_json::json!(0),
        "no emitter-adjacent type may exist in the public API"
    );
    match task_67::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-67 must fail at the absent seam");
            assert!(
                how.contains("no audit event emitter"),
                "the 'how' must name the missing emitter: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-67 passed: an audit event emitter was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
