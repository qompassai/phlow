// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-155 (hostile frame rejection).
//!
//! Five adversarial cases: a 10,000-deep nesting bomb is rejected at
//! the depth bound while a 32-deep envelope still parses; a 100 MB
//! frame is rejected on the 1 MB bound before UTF-8 validation;
//! truncated and invalid-UTF-8 frames produce typed errors; five
//! hostile fixtures fire zero panics under the panic trap and the
//! connection stays up and usable; a counting global allocator proves
//! the read path allocates at most a few KiB per hostile fixture (the
//! 100 MB frame is rejected pre-buffering); the adapted wire module
//! carries the maddada attribution.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_155;
use phlow_gauntlet::wire::EditorSocket;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

static ALLOC_LIVE: AtomicUsize = AtomicUsize::new(0);
static ALLOC_PEAK: AtomicUsize = AtomicUsize::new(0);
/// Serializes the allocator-measuring tests: only one runs at a time,
/// so the peak counter reflects a single test's allocations.
static TEST_LOCK: Mutex<()> = Mutex::new(());

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = ALLOC_LIVE.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            ALLOC_PEAK.fetch_max(live, Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOC_LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        unsafe { System.dealloc(ptr, layout) };
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn check_case(case: &str) -> CaseReport {
    let report = task_155::run_case(case)
        .unwrap_or_else(|e| panic!("task-155 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-155 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// A1: 10,000-deep nesting → DepthExceeded at the bound; 32-deep
/// envelope parses.
#[test]
fn nesting_bomb_depth_bound() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let report = check_case("nesting_bomb_depth_bound");
    let m = &report.metrics;
    assert_eq!(m["bomb_depth"].as_u64().unwrap(), 10_000);
    assert_eq!(m["typed"].as_str().unwrap(), "DepthExceeded");
}

/// A2: 100 MB frame → TooLarge on the 1 MB bound before UTF-8
/// validation.
#[test]
fn oversize_rejected_pre_buffering() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let report = check_case("oversize_rejected_pre_buffering");
    let m = &report.metrics;
    assert_eq!(m["frame_bytes"].as_u64().unwrap(), 100 * 1024 * 1024);
    assert_eq!(m["bound"].as_u64().unwrap(), 1_048_576);
    assert_eq!(m["typed"].as_str().unwrap(), "TooLarge");
}

/// A2 continued: truncated → Truncated, bad UTF-8 → Encoding,
/// empty → Malformed.
#[test]
fn truncated_and_encoding_typed() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let report = check_case("truncated_and_encoding_typed");
    let m = &report.metrics;
    assert!(m["truncated_typed"].as_bool().unwrap());
    assert!(m["encoding_typed"].as_bool().unwrap());
    assert!(m["empty_malformed"].as_bool().unwrap());
}

/// Zero panics across all hostile fixtures; the connection survives.
#[test]
fn zero_panics_connection_survives() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let report = check_case("zero_panics_connection_survives");
    let m = &report.metrics;
    assert_eq!(m["fixtures"].as_u64().unwrap(), 5);
    assert_eq!(m["panics"].as_u64().unwrap(), 0);
    assert_eq!(m["rejected"].as_u64().unwrap(), 5);
    assert!(m["connection_up"].as_bool().unwrap());
}

/// Adversarial allocation bound: the fixtures (including the 100 MB
/// frame) are built outside the measured region; reading them through
/// the socket must peak at no more than a few KiB of live heap — the
/// parser rejects hostile input pre-buffering.
#[test]
fn hostile_read_allocates_at_most_kib() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fixtures: Vec<Vec<u8>> = vec![
        task_155::nesting_bomb(10_000),
        task_155::oversize_frame(),
        task_155::truncated_frame(),
        vec![0xFF, 0xFE, b'{', b'}'],
        Vec::new(),
    ];
    let mut socket = EditorSocket::new();
    let live0 = ALLOC_LIVE.load(Ordering::SeqCst);
    ALLOC_PEAK.store(live0, Ordering::SeqCst);
    let mut rejected = 0u32;
    for bytes in &fixtures {
        if socket.read(bytes).is_err() {
            rejected += 1;
        }
    }
    let peak = ALLOC_PEAK.load(Ordering::SeqCst).saturating_sub(live0);
    assert_eq!(rejected, 5, "all hostile fixtures must be rejected");
    assert!(
        peak <= 8_192,
        "hostile read peaked at {peak} bytes live heap, want at most 8 KiB \
         (rejected pre-buffering)"
    );
}

/// License gate: src/wire.rs and the task driver carry the maddada
/// attribution + source commit.
#[test]
fn license_header_present() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let report = check_case("license_header_present");
    assert_eq!(report.metrics["files_checked"].as_u64().unwrap(), 2);
}
