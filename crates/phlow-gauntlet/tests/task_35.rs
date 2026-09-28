//! Integration tests for task-35 (duplicate delivery dedup).
//!
//! The seam is ABSENT: phlow has no internal event bus or dispatcher.
//! Delivery is request/response RPC (`MsgpackTransport`) and Ollama HTTP
//! (`ReqwestTransport`) — no subscribe/broadcast, no `run.finished`
//! event, no consumer, no delivery-keyed dedup state. The four recon
//! cases drive the real crates (no mocks): 2 validation, 2 adversarial.
//!
//! `task_35::run` itself reports `fail` at `"seam"`; that assertion is
//! folded into the last test to keep the 2V/2A count.

use phlow_gauntlet::tasks::task_35;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(case: &str) -> task_35::CaseReport {
    let report = task_35::run_case(case)
        .unwrap_or_else(|e| panic!("task-35: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-35 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-35-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-35 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-35 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; the real transport constructs,
/// and the delivery surface is request/response only — zero event
/// buses, zero subscribe APIs.
#[test]
fn runtime_transports_are_request_response_only() {
    assert_eq!(task_35::ID, "task-35");
    assert_eq!(task_35::NAME, "duplicate delivery dedup");
    assert_eq!(task_35::KIND, TaskKind::Rust);
    assert_eq!(
        task_35::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let report = run_case("runtime_transports_are_request_response_only");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["event_buses"], 0,
        "there must be no event bus:\n{joined}"
    );
    assert_eq!(
        report.metrics["subscribe_apis"], 0,
        "there must be no subscribe API:\n{joined}"
    );
    assert!(
        joined.contains("MsgpackTransport"),
        "evidence must name the real transport:\n{joined}"
    );
}

/// V: the keyed dedup that exists is caller-keyed at submission
/// (`DuplicateNode` on re-admission) — zero delivery-keyed dedups. The
/// key is supplied by the caller, never assigned by a bus at delivery.
#[test]
fn keyed_dedup_is_submission_side_only() {
    let report = run_case("keyed_dedup_is_submission_side_only");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["delivery_keyed_dedups"], 0,
        "there must be no delivery-keyed dedup:\n{joined}"
    );
    assert!(
        joined.contains("DuplicateNode"),
        "evidence must show the submission-side dedup working:\n{joined}"
    );
    assert!(
        joined.contains("caller-supplied at submission"),
        "evidence must state where the key comes from:\n{joined}"
    );
}

// --- adversarial ---

/// A: there is no event consumer to deliver to twice — no
/// `run.finished` event type, no subscription API, no counted side
/// effect. The design's default scenario cannot be staged.
#[test]
fn no_event_consumer_to_deliver_to() {
    let report = run_case("no_event_consumer_to_deliver_to");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["event_consumers"], 0,
        "there must be no event consumer:\n{joined}"
    );
    assert!(
        joined.contains("no consumer"),
        "evidence must state the consumer absence:\n{joined}"
    );
}

/// A: the restart scenario is vacuous — zero delivery-dedup states
/// exist, so there is nothing whose durability could be asserted.
/// Folded in: the task-level `run` reports `fail` at `"seam"`, keeping
/// the 2V/2A count.
#[test]
fn restart_dedup_state_vacuous_and_task_fails_at_seam() {
    let report = run_case("restart_dedup_state_is_vacuous");
    let joined = report.evidence.join("\n");
    assert_eq!(
        report.metrics["delivery_dedup_states"], 0,
        "there must be no delivery-dedup state:\n{joined}"
    );
    match task_35::run(&test_ctx()) {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            assert_eq!(where_, "seam", "task-35 must fail at the absent seam");
            let joined = evidence.join("\n");
            assert!(
                joined.contains("no internal event bus"),
                "evidence must name the absent bus:\n{joined}"
            );
            assert!(
                how.contains("no delivery to duplicate"),
                "the 'how' must state why no duplicate can be staged: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => {
            panic!("task-35 passed: an event bus was invented, not found\nevidence: {evidence:?}")
        }
    }
}
