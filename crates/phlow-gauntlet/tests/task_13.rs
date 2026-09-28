//! tests/task_13.rs — integration tests for task-13 (scheduler overload).
//!
//! Each test compiles the scenario binary (phlow's real scheduler sources,
//! embedded via `#[path]` — no mocks) and runs one overload case against
//! it: 2 validation + 2 adversarial, plus the driver metadata and the
//! end-to-end `run()`.

use phlow_gauntlet::tasks::task_13;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, per-test scratch directory under the OS temp dir.
fn scratch_dir() -> PathBuf {
    let n = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("gauntlet-task13-{}-{n}", std::process::id()))
}

fn test_ctx(work_dir: PathBuf) -> Ctx {
    // nvim_bin / diver_lua_dir are unused by this Rust-kind task; they only
    // need to satisfy Ctx::new's non-empty validation.
    Ctx::new(PathBuf::from("/bin/true"), PathBuf::from("/tmp"), work_dir)
        .expect("test Ctx fields are non-empty")
}

/// Compile the scenario binary into a fresh scratch dir; return the binary path.
fn build_harness() -> PathBuf {
    let work_dir = scratch_dir();
    task_13::ensure_scenario_binary(&work_dir).expect("scenario must compile from phlow sources")
}

/// Run one case; panic with the scenario's own failure detail on error.
fn run_case(binary: &std::path::Path, case: &str) -> task_13::CaseReport {
    let report = task_13::run_case(binary, case).expect("case must produce a verdict");
    assert_eq!(report.case, case, "verdict case mismatch");
    report
}

/// Task metadata is frozen: the stub's ID/NAME/KIND contract is kept.
#[test]
fn task_metadata_unchanged() {
    assert_eq!(task_13::ID, "task-13");
    assert_eq!(task_13::NAME, "scheduler overload");
    assert_eq!(task_13::KIND, TaskKind::Rust);
    assert_eq!(
        task_13::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
}

/// V1: normal load — half the queue capacity admits and completes fully.
#[test]
fn normal_load_completes_all_work() {
    let binary = build_harness();
    let report = run_case(&binary, "normal_load");
    assert!(report.passed, "normal_load failed: {:?}", report.failures);
    assert_eq!(report.metrics["admitted"].as_u64(), Some(8));
    assert_eq!(report.metrics["published"].as_u64(), Some(8));
    assert_eq!(report.metrics["queue_final"].as_u64(), Some(0));
    assert_eq!(report.metrics["queue_capacity"].as_u64(), Some(16));
}

/// V2: a burst at exactly capacity is absorbed, then drains cleanly.
#[test]
fn burst_at_capacity_absorbed() {
    let binary = build_harness();
    let report = run_case(&binary, "burst_at_capacity");
    assert!(
        report.passed,
        "burst_at_capacity failed: {:?}",
        report.failures
    );
    assert_eq!(report.metrics["admitted"].as_u64(), Some(16));
    assert_eq!(report.metrics["queue_peak"].as_u64(), Some(16));
    assert_eq!(report.metrics["published"].as_u64(), Some(16));
    assert_eq!(report.metrics["queue_final"].as_u64(), Some(0));
}

/// A1: 10x overload — excess submissions are rejected with explicit
/// `QueueFull` errors, never silently dropped; the queue stays bounded and
/// every submission is accounted for.
#[test]
fn overload_10x_rejected_with_explicit_errors() {
    let binary = build_harness();
    let report = run_case(&binary, "overload_10x");
    assert!(report.passed, "overload_10x failed: {:?}", report.failures);
    let metrics = &report.metrics;
    assert_eq!(metrics["submitted"].as_u64(), Some(160));
    assert_eq!(metrics["admitted"].as_u64(), Some(16));
    assert_eq!(metrics["rejected_queue_full"].as_u64(), Some(144));
    assert_eq!(metrics["rejected_other"].as_u64(), Some(0));
    // Conservation: admitted + explicitly rejected == submitted. A silent
    // drop would break this identity.
    let accounted = metrics["accounted"].as_u64().expect("accounted metric");
    assert_eq!(accounted, 160, "every submission must be accounted for");
    assert!(
        metrics["queue_peak"].as_u64().expect("queue_peak metric") <= 16,
        "queue must never exceed capacity"
    );
    assert_eq!(metrics["published"].as_u64(), Some(16));
    assert_eq!(metrics["queue_final"].as_u64(), Some(0));
}

/// A2: sustained overload, then the load subsides — the scheduler drains
/// fully and stays responsive (probe admit + publish succeed).
#[test]
fn sustained_overload_then_drain_recovers() {
    let binary = build_harness();
    let report = run_case(&binary, "sustained_overload");
    assert!(
        report.passed,
        "sustained_overload failed: {:?}",
        report.failures
    );
    let metrics = &report.metrics;
    assert_eq!(metrics["rounds"].as_u64(), Some(5));
    assert!(
        metrics["total_admitted"].as_u64().expect("total_admitted") > 0,
        "waves must admit work while overloaded"
    );
    assert!(
        metrics["total_rejected_queue_full"]
            .as_u64()
            .expect("rejected")
            > 0,
        "overload must shed with explicit rejections"
    );
    assert!(
        metrics["queue_peak"].as_u64().expect("queue_peak") <= 16,
        "queue must never exceed capacity"
    );
    assert_eq!(metrics["queue_final"].as_u64(), Some(0));
    assert_eq!(metrics["probe_ok"].as_bool(), Some(true));
}

/// Driver end-to-end: recon + harness + all four cases → Pass with evidence.
#[test]
fn driver_reports_pass_with_evidence() {
    let work_dir = scratch_dir();
    let ctx = test_ctx(work_dir);
    match task_13::run(&ctx) {
        TaskOutcome::Pass { evidence } => {
            assert!(!evidence.is_empty(), "pass must carry evidence");
            let joined = evidence.join("\n");
            assert!(
                joined.contains("QueueFull"),
                "evidence must name the explicit rejection: {joined}"
            );
            assert!(
                joined.contains("no scheduler execution"),
                "evidence must state the no-executor scope: {joined}"
            );
        }
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => {
            panic!(
                "task-13 driver failed at {where_}: {how}\n{}",
                evidence.join("\n")
            );
        }
    }
}
