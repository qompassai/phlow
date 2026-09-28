//! Integration tests for task-88 (A2A error-shape propagation).
//!
//! The seam is ABSENT in rust: no A2A client boundary exists in any
//! phlow rust crate — exact-token scans over every product crate's
//! `src/**/*.rs` find zero hits for `a2a`, for task-state
//! vocabulary, for agent-card handling, and for the A2A JSON-RPC
//! methods (`tasks/send`, `message/send`, `tasks/get`,
//! `tasks/cancel`). The only A2A boundary in the workspace is
//! diver's Lua (`ai.a2a.client` / `ai.a2a.tasks`, exercised by
//! task-07), where unknown task-state strings are IGNORED by the
//! task state machine rather than rejected as protocol violations —
//! diver-owned, flagged.
//!
//! Four cases — 2 validation, 2 adversarial — each a bounded source
//! recon that fails closed (premise changed) if A2A vocabulary ever
//! appears in a product crate. The driver reports the honest `fail`
//! at `"seam"`.
//!
//! Whether phlow should gain a rust A2A client boundary with typed
//! error-shape propagation is a product decision for Matt — banked,
//! not implemented on gauntlet authority.

use phlow_gauntlet::tasks::task_88;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

fn ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    )
    .unwrap_or_else(|e| panic!("task-88: cannot build Ctx: {e}"))
}

/// Unwrap the expected `fail` at `"seam"`, or panic with the details.
fn fail_at_seam(outcome: TaskOutcome) -> (String, String, Vec<String>) {
    match outcome {
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => (where_, how, evidence),
        TaskOutcome::Pass { evidence } => panic!(
            "task-88 passed: an A2A boundary was invented, not found\nevidence: {evidence:?}"
        ),
    }
}

// --- validation ---

/// V1: no A2A client boundary exists in any rust product crate — the
/// exact-token scan finds zero hits. The task-level driver then runs
/// all four cases and reports the honest seam failure: the rust A2A
/// boundary product decision is banked in the task-level `how`, not
/// implemented on gauntlet authority.
#[test]
fn no_a2a_client_boundary() {
    assert_eq!(task_88::ID, "task-88");
    assert_eq!(task_88::NAME, "A2A error-shape propagation");
    assert_eq!(task_88::KIND, TaskKind::Rust);
    assert_eq!(task_88::CASES.len(), 4, "2 validation + 2 adversarial");
    let report = task_88::run_case("no_a2a_client_boundary")
        .unwrap_or_else(|e| panic!("task-88 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-a2a-boundary case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["hits"], 0);
    // Task-level: the driver fails at the seam (not a pass), and the
    // `how` banks the product decision for Matt.
    let (where_, how, _) = fail_at_seam(task_88::run(&ctx()));
    assert_eq!(where_, "seam", "task-88 must fail at the seam");
    assert!(
        how.contains("seam absent"),
        "the 'how' must name the absent seam: {how}"
    );
    assert!(
        how.contains("Product decision banked"),
        "the 'how' must bank the product decision: {how}"
    );
}

/// V2: no task-state machine exists in rust — zero `task_state` /
/// `taskstate` hits — so unknown state strings have nothing to be
/// rejected by.
#[test]
fn no_task_state_machine() {
    let report = task_88::run_case("no_task_state_machine")
        .unwrap_or_else(|e| panic!("task-88 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-task-state-machine case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["hits"], 0);
}

// --- adversarial ---

/// A1: no agent-card handling exists in rust — a peer's card claims
/// (version, capabilities) cross no rust boundary at all.
#[test]
fn no_agent_card_handling() {
    let report = task_88::run_case("no_agent_card_handling")
        .unwrap_or_else(|e| panic!("task-88 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-agent-card case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no rust code reads an A2A agent card"),
        "evidence must state the absent card reader:\n{joined}"
    );
}

/// A2: none of the A2A JSON-RPC methods exist in rust — error
/// responses to `tasks/send`, `message/stream`, `tasks/get`, or
/// `tasks/cancel` have no rust handler that could preserve (or
/// mishandle) their shape, and attacker-controlled error `data`
/// crosses no rust boundary.
#[test]
fn no_a2a_rpc_methods() {
    let report = task_88::run_case("no_a2a_rpc_methods")
        .unwrap_or_else(|e| panic!("task-88 case failed to run: {e}"));
    assert!(
        report.passed,
        "no-a2a-methods case must hold: {}",
        report.failures.join("; ")
    );
    assert_eq!(report.metrics["hits"], 0);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("attacker-controlled data"),
        "evidence must name the uncrossed data boundary:\n{joined}"
    );
}
