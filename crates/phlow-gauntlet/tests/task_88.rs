//! Integration tests for task-88 (A2A presence audit, D-1059).
//!
//! The original task-88 asserted an ABSENCE: no rust A2A client
//! boundary, no task-state machine, no agent-card handling, no A2A
//! RPC methods — and the driver reported the honest `fail` at
//! `"seam"`. That premise is refuted: `crates/phlow-a2a` now exists.
//! Per D-1059 the task is a PRESENCE AUDIT: it drives the real crate
//! (registry, store, loopback server, client) and asserts the
//! boundary exists and behaves, carrying the original discipline —
//! remote errors typed with code/message/data preserved,
//! attacker-controlled error data untrusted and size-bounded,
//! unknown task states rejected as protocol violations, and remote
//! tasks recorded as data only, never merged into local state.
//!
//! Four cases — 2 validation, 2 adversarial. The task-level driver
//! reports `pass` only when every case's assertions hold against
//! the real crate; these tests encode that verdict.

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

/// Run one case and require it to pass, with its failures as context.
fn passing_report(case: &'static str) -> task_88::CaseReport {
    let report = task_88::run_case(case)
        .unwrap_or_else(|e| panic!("task-88 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-88 case {case} must hold: {}",
        report.failures.join("; ")
    );
    report
}

// --- validation ---

/// V1: registry + handshake. Registering an `ExternalAgent` in an
/// `ExternalRegistry` makes it reachable: `client_for` builds the
/// client, `fetch_card` returns the served card (with the bound URL
/// rewritten in), `send_message` completes a task end-to-end, and
/// `get_task` shows the terminal state. The task-level driver then
/// runs all four cases and reports `pass`.
#[test]
fn registry_and_handshake() {
    assert_eq!(task_88::ID, "task-88");
    assert_eq!(task_88::NAME, "A2A presence audit");
    assert_eq!(task_88::KIND, TaskKind::Rust);
    assert_eq!(task_88::CASES.len(), 4, "2 validation + 2 adversarial");
    assert_eq!(
        task_88::CASES,
        [
            "registry_and_handshake",
            "store_roundtrip",
            "unknown_state_rejected",
            "error_shape_typed_bounded"
        ]
    );
    let report = passing_report("registry_and_handshake");
    assert_eq!(report.metrics["agents_registered"], 1);
    assert_eq!(report.metrics["card_name"], "task-88-audit-agent");
    assert_eq!(report.metrics["task_state"], "TASK_STATE_COMPLETED");
    assert_eq!(report.metrics["artifacts"], 1);
    assert_eq!(report.metrics["fetched_state"], "TASK_STATE_COMPLETED");
    // Task-level: the whole audit passes against the real crate.
    match task_88::run(&ctx()) {
        TaskOutcome::Pass { .. } => {}
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-88 audit must pass: fail at {where_}: {how}\nevidence: {evidence:?}"),
    }
}

/// V2: store round-trip. `TaskStore` create -> append -> legal
/// transitions -> artifact -> get reflects history and artifacts;
/// cancel reaches the terminal `Canceled` state; the illegal
/// `Submitted -> Completed` skip is rejected with a typed
/// `StoreError::InvalidTransition`.
#[test]
fn store_roundtrip() {
    let report = passing_report("store_roundtrip");
    assert_eq!(report.metrics["tasks_in_store"], 2);
    assert_eq!(report.metrics["history_len"], 2);
    assert_eq!(report.metrics["artifacts"], 1);
    assert_eq!(report.metrics["cancelled_state"], "TASK_STATE_CANCELED");
    assert_eq!(report.metrics["completed_chain_ok"], true);
}

// --- adversarial ---

/// A1: unknown task states are rejected as protocol violations at
/// every layer: `TaskState::from_name` returns `None`, the serde
/// decode path refuses a task payload carrying an unknown state,
/// and at the wire a `ListTasks` filter with an unknown state gets
/// JSON-RPC error -32602 rather than a silently unfiltered list.
#[test]
fn unknown_state_rejected() {
    let report = passing_report("unknown_state_rejected");
    assert_eq!(report.metrics["unknown_names_rejected"], 4);
    assert_eq!(report.metrics["state_decode_rejected"], true);
    assert_eq!(report.metrics["task_decode_rejected"], true);
    assert_eq!(report.metrics["wire_error_code"], -32602);
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("wire: ListTasks status=TASK_STATE_BOGUS"),
        "evidence must record the wire-level rejection:\n{joined}"
    );
}

/// A2: remote errors arrive typed and bounded. A real server's
/// unknown-task error reaches the client as `A2aError::Rpc` with
/// code -32001, message, and `TASK_NOT_FOUND` data preserved; a
/// fake peer's attacker-shaped error `data` round-trips verbatim as
/// data in the typed error; an error response larger than the
/// client's response cap is refused as a transport error; and a
/// fetched remote task never enters or mutates a local `TaskStore`.
#[test]
fn error_shape_typed_bounded() {
    let report = passing_report("error_shape_typed_bounded");
    assert_eq!(report.metrics["rpc_code"], -32001);
    assert_eq!(report.metrics["rpc_data_preserved"], true);
    assert_eq!(report.metrics["attacker_data_verbatim"], true);
    assert_eq!(report.metrics["oversized_outcome"], "transport");
    assert_eq!(report.metrics["remote_isolated_from_local_store"], true);
    let offered = report.metrics["oversized_offered_bytes"]
        .as_u64()
        .unwrap_or(0);
    let cap = report.metrics["response_bytes_max"].as_u64().unwrap_or(0);
    assert!(
        offered > cap && cap > 0,
        "the oversized check must offer more than the cap: offered={offered} cap={cap}"
    );
}
