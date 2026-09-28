//! Integration tests for task-38 (ReDoS guard, Rust).
//!
//! There is NO untrusted-regex evaluation seam in the phlow workspace:
//! no member depends on a regex crate, and a recursive `.rs` source
//! scan finds no regex API tokens. The catastrophic backtracking pattern
//! has no evaluator target, and no untrusted config pattern has a sink.
//! The driver is an audit-only driver and reports the honest
//! `Fail { where: "seam" }`. These tests: 2 validation + 2 adversarial —
//! three drive individual cases through `run_case` with the same metrics
//! JSON a harness would collect, and the last one folds in the
//! task-level verdict.
//!
//! Product decision (banked for Matt): if untrusted regex/pattern
//! evaluation is ever introduced, require linear-time evaluation (or a
//! typed timeout) plus pattern allow-list/complexity validation.

use phlow_gauntlet::tasks::task_38;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_38::CaseReport {
    let report = task_38::run_case(case)
        .unwrap_or_else(|e| panic!("task-38: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-38 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-38-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-38 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-38 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; no workspace member (transitively)
/// pulls a pattern-matching crate — `members_pulling_pattern_crates` is
/// 0. The metrics name the transitive regex-crate chain that DOES exist
/// (termwiz via ratatui-termwiz, terminal-escape parsing), which is
/// registry-side and unreachable from phlow input paths.
#[test]
fn workspace_members_pull_no_regex() {
    assert_eq!(task_38::ID, "task-38");
    assert_eq!(task_38::NAME, "ReDoS guard");
    assert_eq!(task_38::KIND, TaskKind::Rust);
    assert_eq!(
        task_38::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("workspace_members_pull_no_regex");
    assert_eq!(
        report.metrics["members_pulling_pattern_crates"],
        serde_json::json!(0),
        "no member may pull a pattern-matching crate"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("termwiz"),
        "metrics evidence must name the transitive chain that exists:\n{evidence}"
    );
}

/// V: no regex API usage in the workspace sources — the recursive `.rs`
/// scan for compiled API/import tokens (assembled at runtime so the
/// probe cannot self-match) finds zero hits, so no application call
/// site can feed untrusted input to a backtracking matcher.
#[test]
fn no_regex_api_usage_in_sources() {
    let report = run_case("no_regex_api_usage_in_sources");
    assert_eq!(
        report.metrics["usage_hits"],
        serde_json::json!(0),
        "no regex API usage may exist in sources"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("hits: 0"),
        "evidence must show the hit count:\n{evidence}"
    );
}

// --- adversarial ---

/// A: the catastrophic pattern is a loaded weapon with no target —
/// `evaluators_found` is 0. This is the adversarial case: it asserts
/// the metric that matters rather than a simulated ReDoS timeout the
/// workspace cannot produce.
#[test]
fn catastrophic_pattern_has_no_evaluator() {
    let report = run_case("catastrophic_pattern_has_no_evaluator");
    assert_eq!(
        report.metrics["evaluators_found"],
        serde_json::json!(0),
        "the pattern must have no evaluator target"
    );
    let evidence = report.evidence.join("\n");
    assert!(
        evidence.contains("none exists in phlow code"),
        "evidence must state the weapon has no target:\n{evidence}"
    );
}

/// A: untrusted config patterns have no evaluation sink (check-name
/// validation is hand-rolled); and the task-level verdict is the honest
/// `Fail { where: "seam" }` — the seam is absent as designed.
#[test]
fn untrusted_config_patterns_have_no_sink_and_task_fails_at_seam() {
    let report = run_case("untrusted_config_patterns_have_no_sink");
    assert_eq!(
        report.metrics["phlow_config_usage_hits"],
        serde_json::json!(0),
        "no untrusted config pattern may reach a sink"
    );
    match task_38::run(&test_ctx()) {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-38 must fail at the absent seam");
            assert!(
                how.contains("no regex/pattern evaluation"),
                "the 'how' must name the missing seam: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-38 passed: a regex evaluation seam was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
