//! tests/task_47.rs — file descriptor exhaustion (task-47, rust).
//!
//! Four integration tests, 50/50 validation/adversarial:
//!
//! - V1 `bounded_fanout_stays_under_the_cap`: 5 concurrent fires all
//!   complete cleanly; nothing is dropped.
//! - V2 `every_spawn_path_is_accounted_for`: the source walk finds
//!   exactly the gated hook spawn, the synchronous checks spawn, and
//!   one test-only CLI spawn.
//! - A1 `adversarial_storm_cannot_open_a_thousand_handles`: 10,000
//!   fires shed load as recorded dropped outcomes; live children never
//!   exceed `HOOK_CONCURRENT_MAX`.
//! - A2 `adversarial_crashing_tools_leave_no_zombies`: crashing tools
//!   reap cleanly and permits return to baseline, then the task driver
//!   runs end-to-end and passes.

use phlow_gauntlet::tasks::task_47::{self, DriverError};
use phlow_gauntlet::{Ctx, TaskOutcome};
use std::path::PathBuf;
use std::sync::Mutex;

/// The storm case measures process-GLOBAL /proc counts (children, FDs),
/// so the subprocess-driving tests in this binary must not run
/// concurrently with each other — otherwise one test's children inflate
/// another's measurement. This lock is test-harness hygiene, not part of
/// the system under test: each case drives its own `HookManager` with
/// its own semaphore pair.
static SUBPROCESS_LOCK: Mutex<()> = Mutex::new(());

/// Build a `Ctx` from the live crate directory, like the other gauntlet
/// tests do.
fn test_ctx() -> Ctx {
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-47-run-{}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: rust task, no nvim involved"),
        PathBuf::from("unused: rust task, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

#[test]
fn bounded_fanout_stays_under_the_cap() {
    let _guard = SUBPROCESS_LOCK.lock().expect("subprocess lock poisoned");
    let report = task_47::run_case("normal_fanout_runs").expect("V1 case must run to completion");
    assert!(report.passed, "V1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["dropped"], serde_json::json!(0));
    assert_eq!(report.metrics["clean_runs"], serde_json::json!(5));
}

#[test]
fn every_spawn_path_is_accounted_for() {
    let report = task_47::run_case("spawn_paths_go_through_limiter")
        .expect("V2 case must run to completion");
    assert!(report.passed, "V2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["spawn_sites"], serde_json::json!(3));
    assert_eq!(report.metrics["gated_fanout"], serde_json::json!(1));
    assert_eq!(report.metrics["test_only"], serde_json::json!(1));
}

#[test]
fn adversarial_storm_cannot_open_a_thousand_handles() {
    let _guard = SUBPROCESS_LOCK.lock().expect("subprocess lock poisoned");
    let report =
        task_47::run_case("storm_10000_spawns_capped").expect("A1 case must run to completion");
    assert!(report.passed, "A1 failed: {:?}", report.failures);
    assert_eq!(report.metrics["spawn_errors"], serde_json::json!(0));
    assert!(
        report.metrics["max_concurrent_children"]
            .as_u64()
            .expect("max_concurrent_children is a number")
            <= 8,
        "concurrent children must stay under HOOK_CONCURRENT_MAX"
    );
}

#[test]
fn adversarial_crashing_tools_leave_no_zombies() {
    let _guard = SUBPROCESS_LOCK.lock().expect("subprocess lock poisoned");
    let report = task_47::run_case("crashed_tools_return_to_baseline")
        .expect("A2 case must run to completion");
    assert!(report.passed, "A2 failed: {:?}", report.failures);
    assert_eq!(report.metrics["baseline_restored"], serde_json::json!(true));

    let outcome = task_47::run(&test_ctx());
    match outcome {
        TaskOutcome::Pass { evidence } => {
            assert!(evidence.iter().any(|line| line.contains("HookManager")));
        }
        other => panic!("task-47 should pass, got: {other:?}"),
    }
    let _: Option<DriverError> = None;
}
