//! tests/task_53.rs — quorum reads and writes (task-53, rust).
//!
//! Four integration tests, 50/50 validation/adversarial. The task
//! verdict is `fail` at `"seam"`: no replicated state store exists in
//! any phlow crate — the replication vocabulary scan returns zero, the
//! real stores are single-node, the real `Scheduler` shows the
//! degenerate quorum (W=1, R=1, N=1), partitions are vacuous, and no
//! per-record version exists for read-repair.
//!
//! - V1 `single_node_stores_located`: the replication vocabulary scan
//!   returns zero; the real stores are documented as single-node.
//! - V2 `single_node_read_write_semantics`: drive the real `Scheduler`
//!   — one local write, one local read, no quorum knobs on any API.
//! - A1 `partition_is_vacuous`: no peer set, no partition primitive.
//! - A2 `no_version_for_stale_read_prevention`: task-31's finding
//!   re-verified — only the `schema_version` constant, no per-record
//!   version for read-repair.

use phlow_gauntlet::tasks::task_53;
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;

/// Build a `Ctx` from the live crate directory, like the other gauntlet
/// tests do.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-53-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: rust task, no nvim involved"),
        PathBuf::from("unused: rust task, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

#[test]
fn single_node_stores_located() {
    assert_eq!(task_53::ID, "task-53");
    assert_eq!(task_53::NAME, "quorum reads and writes");
    assert_eq!(task_53::CASES.len(), 4, "2 validation + 2 adversarial");
    let report =
        task_53::run_case("single_node_stores_located").expect("V1 case must run to completion");
    assert!(report.passed, "V1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["replication_hits"], serde_json::json!(0));
}

#[test]
fn single_node_read_write_semantics() {
    let report = task_53::run_case("single_node_read_write_semantics")
        .expect("V2 case must run to completion");
    assert!(report.passed, "V2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["replicas"], serde_json::json!(1));
    assert_eq!(report.metrics["write_quorum"], serde_json::json!(1));
    assert_eq!(report.metrics["read_quorum"], serde_json::json!(1));
}

#[test]
fn partition_is_vacuous() {
    let report = task_53::run_case("partition_is_vacuous").expect("A1 case must run to completion");
    assert!(report.passed, "A1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["peer_partition_hits"], serde_json::json!(0));
}

#[test]
fn no_version_for_stale_read_prevention() {
    let report = task_53::run_case("no_version_for_stale_read_prevention")
        .expect("A2 case must run to completion");
    assert!(report.passed, "A2 failed: {:?}", report.failures);
    assert_eq!(
        report.metrics["per_record_version"],
        serde_json::json!(false)
    );

    // The task-level verdict is the honest fail at the absent seam.
    let outcome = task_53::run(&test_ctx());
    match outcome {
        TaskOutcome::Fail { where_, how, .. } => {
            assert_eq!(where_, "seam", "task-53 must fail at the absent seam");
            assert!(
                how.contains("no replicated state store exists"),
                "the 'how' must name the absent seam: {how}"
            );
        }
        TaskOutcome::Pass { evidence } => panic!(
            "task-53 passed: quorum machinery was invented, not found\nevidence: {evidence:?}"
        ),
    }
}
