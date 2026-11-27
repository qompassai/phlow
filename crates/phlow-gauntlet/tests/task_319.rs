//! tests/task_319.rs — context API: pinned tally and archive retrieval
//! (task-319, rust).
//!
//! Four integration tests, 50/50 validation/adversarial, plus the
//! task-level run:
//!
//! - V1 `collapse_retrieve_roundtrip`: collapse archives the originals
//!   and retrieve returns them byte-for-byte in order.
//! - V2 `summary_insufficient_retrieval_required`: the summary omits a
//!   figure by construction; the tally can complete only through the
//!   archive — retrieval is measured, not assumed.
//! - A1 `injection_cannot_reach_tally_or_task`: an injection-shaped
//!   tool result under a compaction storm changes neither the tally
//!   nor the task; it survives only as data.
//! - A2 `compaction_storm_stays_within_named_bounds`: a fill-and-compact
//!   storm past the archive cap respects every named bound.

use phlow_gauntlet::tasks::task_319;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` from the live crate directory, like the other gauntlet
/// tests do.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-319-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: rust task, no nvim involved"),
        PathBuf::from("unused: rust task, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

#[test]
fn collapse_retrieve_roundtrip() {
    let report = task_319::run_case("collapse_retrieve_roundtrip_preserves_originals")
        .expect("V1 case must run to completion");
    assert!(report.passed, "V1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["archived_events"], serde_json::json!(4));
    assert_eq!(report.metrics["retrieved_events"], serde_json::json!(4));
}

#[test]
fn summary_insufficient_retrieval_required() {
    let report = task_319::run_case("summary_insufficient_retrieval_required")
        .expect("V2 case must run to completion");
    assert!(report.passed, "V2 failed: {:?}", report.failures);
    assert_eq!(
        report.metrics["retrieval_required"],
        serde_json::json!(true)
    );
    assert_eq!(
        report.metrics["pre_retrieval_tally"],
        serde_json::json!(1450)
    );
    assert_eq!(report.metrics["final_tally"], serde_json::json!(2440));
}

#[test]
fn injection_cannot_reach_tally_or_task() {
    let report = task_319::run_case("injection_cannot_reach_tally_or_task")
        .expect("A1 case must run to completion");
    assert!(report.passed, "A1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["tally_value"], serde_json::json!("1170"));
    assert_eq!(
        report.metrics["injection_kept_as_data"],
        serde_json::json!(true)
    );
}

#[test]
fn compaction_storm_stays_within_named_bounds() {
    let report = task_319::run_case("compaction_storm_stays_within_named_bounds")
        .expect("A2 case must run to completion");
    assert!(report.passed, "A2 failed: {:?}", report.failures);
    let archives = report.metrics["archive_ranges"]
        .as_u64()
        .expect("archive_ranges is a number");
    let cap = report.metrics["archive_cap"]
        .as_u64()
        .expect("archive_cap is a number");
    assert!(archives <= cap, "archive {archives} past cap {cap}");
}

#[test]
fn task_319_full_run_passes() {
    let outcome = task_319::run(&test_ctx());
    assert!(
        matches!(outcome, TaskOutcome::Pass { .. }),
        "task-319 must pass end to end: {outcome:?}"
    );
}
