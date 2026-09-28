//! Integration tests for task-25 (backpressure propagation).
//!
//! The driver exercises the REAL producer/consumer channel in
//! `phlow_runtime::transport::MsgpackTransport`: the bounded
//! (`WORKER_QUEUE_CAPACITY = 1`) mpsc channel between the producer side
//! of `exec` and the current-thread tokio worker owning the socket. A
//! scripted msgpack-RPC peer (rmpv, the transport's own codec) plays the
//! slow consumer; the producer must park, never buffer unboundedly, and
//! fail explicitly on a black hole.
//! 50/50 split: 2 validation, 2 adversarial.
//!
//! Each test gets its own socket directory (pid + process-local
//! counter): parallel tests never share a peer socket.

use phlow_gauntlet::tasks::task_25;
use phlow_gauntlet::{Ctx, TaskKind, TaskOutcome};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-local sequence so concurrent tests never share a socket dir.
static SOCKDIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// A fresh socket directory for one test.
fn socket_dir(case: &str) -> PathBuf {
    let seq = SOCKDIR_SEQ.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "gauntlet-task-25-{case}-{}-{seq}",
        std::process::id()
    ))
}

/// Run one case; unwrap its report or fail with the case's own detail.
fn run_case(dir: &Path, case: &str) -> task_25::CaseReport {
    let report = task_25::run_case(dir, case)
        .unwrap_or_else(|e| panic!("task-25: run_case '{case}' errored: {e}"));
    assert_eq!(report.case, case, "verdict case mismatch");
    assert!(
        report.passed,
        "task-25 case '{case}' failed: {:?}",
        report.failures
    );
    report
}

/// Build a `Ctx` for the task-level run. This task drives no Neovim and no
/// nvim-lua driver, so the binary/diver paths are documented placeholders;
/// `Ctx::new` only requires them to be non-empty.
fn test_ctx() -> Ctx {
    let seq = SOCKDIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let work_dir =
        std::env::temp_dir().join(format!("gauntlet-task-25-run-{}-{seq}", std::process::id()));
    Ctx::new(
        PathBuf::from("unused: task-25 is TaskKind::Rust, no nvim involved"),
        PathBuf::from("unused: task-25 is TaskKind::Rust, no diver lua involved"),
        work_dir,
    )
    .expect("gauntlet test: Ctx::new rejected non-empty paths")
}

// --- validation ---

/// V: metadata contract pins the task; matched rates serve 5 sequential
/// execs with exact canned replies and small latencies. Folded in: the
/// task-level `run` passes end-to-end (all four cases in one driver run).
#[test]
fn matched_rates_serve_exact_replies() {
    assert_eq!(task_25::ID, "task-25");
    assert_eq!(task_25::NAME, "backpressure propagation");
    assert_eq!(task_25::KIND, TaskKind::Rust);
    assert_eq!(
        task_25::CASES.len(),
        4,
        "2 validation + 2 adversarial cases"
    );
    let dir = socket_dir("matched_rates");
    let report = run_case(&dir, "matched_rates");
    let execs = report.metrics["execs"].as_u64().unwrap_or(0);
    let worst_ms = report.metrics["worst_latency_ms"]
        .as_u64()
        .unwrap_or(u64::MAX);
    assert_eq!(execs, 5, "expected 5 execs, got {execs}");
    assert!(
        worst_ms <= 5000,
        "worst latency {worst_ms} ms exceeds the local bound"
    );
    match task_25::run(&test_ctx()) {
        TaskOutcome::Pass { .. } => {}
        TaskOutcome::Fail {
            where_,
            how,
            evidence,
        } => panic!("task-25 end-to-end run failed at '{where_}': {how}\n{evidence:?}"),
    }
}

/// V: a slow consumer (2s reply delay) parks the producer — the call takes
/// at least the delay, succeeds with the intact reply, and never fails or
/// buffers.
#[test]
fn slow_consumer_parks_the_producer() {
    let dir = socket_dir("slow_consumer_parks");
    let report = run_case(&dir, "slow_consumer_parks");
    let parked = report.metrics["parked_secs"].as_f64().unwrap_or(0.0);
    assert!(
        (2.0..=25.0).contains(&parked),
        "producer must park ~2s behind the consumer, parked {parked:.2}s"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("blocked instead of failing or buffering"),
        "evidence must state the parking behavior:\n{joined}"
    );
}

// --- adversarial ---

/// A: the consumer stalls completely for 30s with 4 producers parked in
/// flight. Every producer rides out the stall, memory stays flat (the
/// capacity-1 channel bounds buffering), and no message is lost when the
/// consumer resumes.
#[test]
fn full_stall_30s_resumes_with_flat_memory() {
    let dir = socket_dir("stall_30s_resumes_cleanly");
    let report = run_case(&dir, "stall_30s_resumes_cleanly");
    let producers = report.metrics["producers"].as_u64().unwrap_or(0);
    let wall = report.metrics["total_wall_secs"].as_f64().unwrap_or(0.0);
    let growth = report.metrics["rss_growth_mib"]
        .as_f64()
        .unwrap_or(f64::MAX);
    assert_eq!(producers, 4, "expected 4 producers, got {producers}");
    assert!(
        (30.0..=75.0).contains(&wall),
        "total wall {wall:.1}s must bracket the 30s stall"
    );
    assert!(
        growth < 128.0,
        "RSS grew {growth:.1} MiB during the stall: unbounded buffering"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("no messages lost"),
        "evidence must state zero message loss:\n{joined}"
    );
}

/// A: a black-hole peer makes `exec` fail EXPLICITLY with
/// `TransportError::Timeout` at the deadline — no hang. Then the same
/// transport recovers: a healthy peer on the same socket path serves the
/// next exec via reconnect.
#[test]
fn black_hole_times_out_explicitly_then_recovers() {
    let dir = socket_dir("timeout_is_explicit");
    let report = run_case(&dir, "timeout_is_explicit");
    let timed_out = report.metrics["timeout_secs"].as_f64().unwrap_or(0.0);
    let recovered = report.metrics["recover_secs"].as_f64().unwrap_or(f64::MAX);
    assert!(
        (2.0..=10.0).contains(&timed_out),
        "timeout must fire near the 2s deadline, took {timed_out:.2}s"
    );
    assert!(
        recovered < 10.0,
        "post-timeout recovery took {recovered:.2}s"
    );
    let joined = report.evidence.join("\n");
    assert!(
        joined.contains("TransportError::Timeout"),
        "evidence must name the explicit timeout error:\n{joined}"
    );
    assert!(
        joined.contains("reconnected fresh"),
        "evidence must state the worker reconnected:\n{joined}"
    );
}
