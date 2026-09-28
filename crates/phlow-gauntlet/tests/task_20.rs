//! Integration tests for task-20 (deadline expiry / at-most-once).
//!
//! 50/50 split against phlow's real host-owned scheduler
//! (`phlow_experiment::control_plane::Scheduler`):
//!
//! - V1: CLI golden output — `gauntlet run task-20` prints one JSON report
//!   whose evidence shows an expired deadline recorded as `timed_out`,
//!   with the verdict name pairwise distinct from `failed`/`cancelled`.
//! - V2: a late cancellation cannot resurrect a terminal node, and a late
//!   result for a cancelled run is rejected (`RunCancelled`).
//! - A1: at-most-once — a second publication is rejected
//!   (`DuplicateResult`); the recorded digest and generation never change.
//! - A2: stale generations and non-terminal states cannot publish
//!   (`StaleGeneration`, `NotTerminal`); rejected publishes mutate nothing.

use phlow_gauntlet::tasks::task_20;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` for one test. This task drives no Neovim and no nvim-lua
/// driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    Ctx::new(
        PathBuf::from("unused: task-20 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-20 is TaskKind::Rust, no diver lua involved"),
        std::env::temp_dir().join("gauntlet-task-20"),
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

/// Run the driver; unwrap the Pass outcome or fail with the driver's own
/// evidence attached.
fn run_pass() -> Vec<String> {
    match task_20::run(&test_ctx()) {
        TaskOutcome::Pass { evidence } => evidence,
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-20 driver failed at {where_}: {how}\nevidence: {evidence:?}"),
    }
}

/// The `gauntlet` CLI binary under test, provided by Cargo to integration
/// tests of this package.
fn gauntlet_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gauntlet"))
}

// --- validation ---

/// V: CLI golden output — the operator reads `timed_out` for the expired
/// deadline, and the report proves at a glance it is not `failed` and not
/// `cancelled` (pairwise-distinct verdict names).
#[test]
fn cli_report_shows_timed_out_distinct_from_failed_and_cancelled() {
    assert_eq!(task_20::ID, "task-20");
    assert_eq!(task_20::NAME, "deadline expiry and at-most-once");
    assert_eq!(task_20::KIND, TaskKind::Rust);
    let work_dir = std::env::temp_dir().join("gauntlet-task-20-cli-golden");
    // Rust-kind task: the CLI still requires the path flags, but they are
    // documented placeholders here — no nvim is spawned.
    let output = std::process::Command::new(gauntlet_bin())
        .arg("run")
        .arg("task-20")
        .arg("--nvim-bin")
        .arg("/bin/true")
        .arg("--diver-lua")
        .arg("/tmp")
        .arg("--work-dir")
        .arg(&work_dir)
        .output()
        .expect("task-20: cannot run gauntlet CLI");
    assert!(
        output.status.success(),
        "gauntlet run task-20 exited nonzero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("task-20: CLI output is not JSON");
    assert_eq!(report["id"], "task-20");
    assert_eq!(report["kind"], "rust");
    assert_eq!(report["outcome"], "pass");
    let evidence: Vec<String> = report["evidence"]
        .as_array()
        .expect("evidence is an array")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let joined = evidence.join("\n");
    assert!(
        joined.contains("expired deadline published as timed_out (terminal, digest recorded once)"),
        "expected the timed_out verdict in the CLI report, got:\n{joined}"
    );
    let view = evidence
        .iter()
        .find(|line| line.starts_with("operator view: verdict=timed_out"))
        .expect("expected a timed_out operator-view line in the CLI report");
    assert!(
        view.contains("distinct from failed/cancelled/timed_out peers: true"),
        "operator view must prove at-a-glance distinctness, got: {view}"
    );
}

/// V: a late cancellation moves nothing on a terminal node, and a late
/// result for a cancelled run is rejected — the node keeps its state.
#[test]
fn late_cancel_cannot_resurrect_and_late_result_rejected() {
    let joined = run_pass().join("\n");
    assert!(
        joined.contains("cancel_run on a timed_out run transitioned 0 nodes; node still timed_out"),
        "expected the late cancel to move nothing, got:\n{joined}"
    );
    assert!(
        joined.contains("late result for a cancelled run rejected with RunCancelled"),
        "expected RunCancelled on the late result, got:\n{joined}"
    );
    assert!(
        joined.contains("cancelled node kept its state after the rejected late result"),
        "expected the cancelled node unchanged, got:\n{joined}"
    );
}

// --- adversarial ---

/// A: the second publication is rejected with `DuplicateResult`; the
/// recorded digest, generation, and node state are provably unchanged —
/// at-most-once holds under a double-complete attempt.
#[test]
fn double_publish_rejected_and_record_unchanged() {
    let joined = run_pass().join("\n");
    assert!(
        joined.contains("second publication rejected with DuplicateResult"),
        "expected DuplicateResult, got:\n{joined}"
    );
    assert!(
        joined.contains("digest and generation unchanged; node still succeeded exactly once"),
        "expected the record provably unchanged, got:\n{joined}"
    );
}

/// A: a stale generation and a non-terminal state both fail to publish,
/// and the rejections mutate nothing — the node is still admitted with
/// zero published results.
#[test]
fn stale_generation_and_nonterminal_publish_rejected() {
    let joined = run_pass().join("\n");
    assert!(
        joined.contains("stale-generation publish rejected with StaleGeneration"),
        "expected StaleGeneration, got:\n{joined}"
    );
    assert!(
        joined.contains("non-terminal publish rejected with NotTerminal"),
        "expected NotTerminal, got:\n{joined}"
    );
    assert!(
        joined.contains("node still admitted, published_count 0 after both rejections"),
        "expected no mutation from rejected publishes, got:\n{joined}"
    );
}
