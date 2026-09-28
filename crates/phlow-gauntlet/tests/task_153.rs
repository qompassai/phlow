// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-153 (bounded parse cost).
//!
//! Three validation cases: whitespace and key-order variants of one
//! envelope parse to identical bytes (canonical equivalence);
//! 10,000 parses complete with a ManualClock delta of exactly one
//! tick per parse and a byte-scan cost of exactly 1 per byte
//! (bytes_scanned == total input len) with a size sweep; a counting global allocator proves peak live heap across
//! 10,000 parses stays within `MAX_ENVELOPE_BYTES` and does not leak;
//! the adapted wire module carries the maddada attribution.

use phlow_gauntlet::bounty::clock::ManualClock;
use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::tasks::task_153;
use phlow_gauntlet::wire::MAX_ENVELOPE_BYTES;
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
    let report = task_153::run_case(case)
        .unwrap_or_else(|e| panic!("task-153 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-153 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

/// V1: whitespace and key-order variants parse to identical bytes.
#[test]
fn canonical_equivalence() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let report = check_case("canonical_equivalence");
    let m = &report.metrics;
    assert_eq!(m["spellings"].as_u64().unwrap(), 4);
    assert!(m["identical"].as_bool().unwrap());
}

/// V2: 10,000 parses → 10,000 ticks, byte-scan cost exactly 1 per
/// byte (scanned == total input len), sweep linear at 4 sizes, the
/// bound-size envelope parses.
#[test]
fn bounded_allocation_linear_time() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let report = check_case("bounded_allocation_linear_time");
    let m = &report.metrics;
    assert_eq!(m["parses"].as_u64().unwrap(), 10_000);
    assert_eq!(m["ticks"].as_u64().unwrap(), 10_000);
    assert!(m["sweep_linear"].as_bool().unwrap());
    assert_eq!(m["bound_bytes"].as_u64().unwrap(), 65_536);
}

/// V3: peak live heap across 10,000 parses ≤ MAX_ENVELOPE_BYTES,
/// and nothing leaks afterwards.
#[test]
fn allocation_bounded_by_max_envelope_bytes() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Build the fixtures outside the measured region.
    let inputs: Vec<Vec<u8>> = (0..10_000usize)
        .map(|i| {
            format!(
                r#"{{"version":1,"kind":"event","id":"e{i:06}","body":{{"i":{i},"msg":"hello","tags":["a","b","c"]}}}}"#
            )
            .into_bytes()
        })
        .collect();
    let mut clock = ManualClock::new(1_700_000_000);
    let live0 = ALLOC_LIVE.load(Ordering::SeqCst);
    ALLOC_PEAK.store(live0, Ordering::SeqCst);
    let (ok, _scanned, _ticks) = task_153::bench_parse(&inputs, &mut clock);
    let peak = ALLOC_PEAK.load(Ordering::SeqCst).saturating_sub(live0);
    let live_end = ALLOC_LIVE.load(Ordering::SeqCst);
    assert_eq!(ok, 10_000, "all 10,000 parses must succeed");
    assert!(
        peak <= MAX_ENVELOPE_BYTES,
        "peak live heap {peak} bytes across 10,000 parses exceeds MAX_ENVELOPE_BYTES ({MAX_ENVELOPE_BYTES})"
    );
    assert!(
        live_end <= live0 + 4096,
        "parse loop leaked: live {live_end} vs baseline {live0}"
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
