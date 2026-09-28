// Copyright (c) maddada
// Ghostex concept adapted from maddada/Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500
// Re-implemented for phlow in Tiger Style Rust; not a verbatim port.

//! Integration tests for task-168 (hostile event injection).
//!
//! Two adversarial cases against
//! [`phlow_gauntlet::state_machine::ingest`]: unknown event variants
//! are rejected at ingestion with the state untouched, and hostile
//! payloads (10 MB string, `u64::MAX`/0 ids, bad percent, oversized
//! reason) are refused with typed errors, zero panics, and no
//! allocation spike — the last asserted here with a real counting
//! global allocator around `ingest`.

use phlow_gauntlet::skillopt::driver::CaseReport;
use phlow_gauntlet::state_machine::{
    EventError, INGEST_ALLOC_BUDGET_BYTES, MAX_LABEL_BYTES, RawEvent, ingest,
};
use phlow_gauntlet::tasks::task_168;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Bytes allocated through the global allocator since the last reset.
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Counting allocator: delegates to [`System`], recording every
/// allocation's size. Test-binary only — the library crate keeps
/// `#![forbid(unsafe_code)]`.
struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: same layout delegated to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOC_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: same layout delegated to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// True when the refusal names the full hostile size and the label bound.
fn oversized_is_10mb(result: &Result<phlow_gauntlet::state_machine::Event, EventError>) -> bool {
    const HOSTILE_BYTES: usize = 10 * 1024 * 1024;
    matches!(
        result,
        Err(EventError::OversizedPayload {
            bytes: HOSTILE_BYTES,
            max: MAX_LABEL_BYTES,
            ..
        })
    )
}

fn check_case(case: &str) -> CaseReport {
    let report = task_168::run_case(case)
        .unwrap_or_else(|e| panic!("task-168 case {case} failed to run: {e}"));
    assert!(
        report.passed,
        "task-168 case {case} failed: {}",
        report.failures.join("; ")
    );
    report
}

// --- adversarial ---

/// A1: unknown variants → `EventError::Unknown`; state untouched.
#[test]
fn unknown_event_variant_rejected() {
    assert_eq!(task_168::ID, "task-168");
    let report = check_case("unknown_event_variant_rejected");
    let m = &report.metrics;
    assert_eq!(m["refusal"].as_str().unwrap(), "Unknown");
    assert_eq!(m["rejected"].as_u64().unwrap(), 6);
    assert!(m["state_unchanged"].as_bool().unwrap());
}

/// A2: hostile payloads → typed errors, zero panics, state untouched.
#[test]
fn hostile_payload_bounds_enforced() {
    let report = check_case("hostile_payload_bounds_enforced");
    let m = &report.metrics;
    assert_eq!(m["panics"].as_u64().unwrap(), 0);
    assert!(m["state_unchanged"].as_bool().unwrap());
    assert_eq!(m["hostile_label_bytes"].as_u64().unwrap(), 10 * 1024 * 1024);
}

/// A2 (allocation half): refusing a 10 MB label must not copy it.
/// The payload is rejected on length before any clone, so the
/// allocation delta across `ingest` stays under the named budget —
/// 160x below the hostile size. The 10 MB fixture is allocated
/// before the counter resets, so only the refusal itself is measured.
#[test]
fn hostile_label_refusal_alloc_bounded() {
    let label = "x".repeat(10 * 1024 * 1024);
    assert_eq!(label.len(), 10 * 1024 * 1024);
    let raw = RawEvent {
        kind: "spawn".to_string(),
        task_id: 2,
        text: label,
        percent: 0,
    };
    ALLOC_BYTES.store(0, Ordering::Relaxed);
    let result = ingest(&raw);
    let delta = ALLOC_BYTES.load(Ordering::Relaxed);
    assert!(
        matches!(
            result,
            Err(EventError::OversizedPayload { field: "label", .. })
        ) && oversized_is_10mb(&result),
        "10MB label must be refused as OversizedPayload, got: {result:?}"
    );
    assert!(
        delta <= INGEST_ALLOC_BUDGET_BYTES,
        "allocation spike: refusing the 10MB label allocated {delta} bytes, \
         budget is {INGEST_ALLOC_BUDGET_BYTES}"
    );
}
