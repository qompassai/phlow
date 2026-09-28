# task-153: lenient envelope parsing

**Kind:** rust (validation) · **Status:** pass → fixed · **Wave:** 25 · **Commit:** pending (wave 25 commit)

## ELI5

Real peers send imperfect bytes: extra spaces, newlines around the message, fields in a different order. A strict parser that chokes on whitespace breaks on contact with reality; a sloppy parser that accepts anything breaks on contact with an attacker. The Ghostex rule is "liberal in what you accept *within typed bounds*": whitespace and key order don't matter, but the shape still has to be a real envelope. This task proves both halves — four spellings of one envelope (whitespace, reordered keys) all parse to the identical value — and that parsing stays cheap: 10,000 parses on a scripted clock, one tick each, every input byte visited exactly once, and a counting allocator proving peak live heap stays within `MAX_ENVELOPE_BYTES` (64 KiB).

## What this task attempts

- **Goal:** imperfect-but-well-formed envelopes parse to the canonical value; 10,000 parses show bounded allocation (≤ `MAX_ENVELOPE_BYTES`) and byte-linear scan cost.
- **Mechanism:** `crates/phlow-gauntlet/src/wire.rs` — `parse_envelope`, `ParseStats.bytes_scanned`; driver `crates/phlow-gauntlet/src/tasks/task_153.rs` (`bench_parse`); integration test `tests/task_153.rs` with a counting global allocator.
- **Success criterion:** 4 spellings → identical `Envelope` and identical canonical JSON; 10,000 parses → 10,000 ticks, `bytes_scanned == total input len`, size sweep linear at 4 sizes, 64 KiB envelope parses; allocator peak ≤ 64 KiB with no leak.
- **Non-goals:** hostile input (task 155); wall-clock timing (gauntlet convention: linearity proven by exact byte counts, not a stopwatch).

## What happened

Passed after test-side fixes. `cargo test -p phlow-gauntlet --test task_153` → 4 passed, 0 failed. The allocation case measured peak live heap across 10,000 parses within the 64 KiB bound with no residual growth; the driver case showed `bytes_scanned` exactly equal to total input length (cost of exactly 1 per byte — the single linear pass) and the sweep linear at 256/512/1024/2048-byte sizes.

## Where it went wrong

- **Stage:** integration test, first gate run.
- **Symptom:** `bounded_allocation_linear_time` panicked on `Option::unwrap()` on `None` at `m["parses"]` — then the shared `TEST_LOCK` mutex poisoned and the other three tests failed at `lock().unwrap()` with `PoisonError`, masking the real failure.
- **Evidence:** `thread 'bounded_allocation_linear_time' panicked at crates/phlow-gauntlet/tests/task_153.rs:79:44: called Option::unwrap() on a None value`; the other three tests then reported `PoisonError`.
- **Root cause:** the test asserted metric keys the driver never emitted (`variants`, `cost_per_byte`, `sweep_max_bytes`). The driver's actual metrics are `spellings`, `identical`, `parses`, `ticks`, `bytes_scanned`, `sweep_linear`, `bound_bytes`. The test was written against an imagined contract, not the driver's. The poison cascade was a second, test-hygiene defect: a bare `lock().unwrap()` turns one failure into four.

## The fix — what changed and why

- **Changed:** `tests/task_153.rs` — metric keys corrected to the driver's real contract (`spellings == 4`, `identical`, `parses == 10_000`, `ticks == 10_000`, `sweep_linear`, `bound_bytes == 65_536`); the module docstring corrected from "cost of 2 per byte" to exactly 1 per byte (what the driver actually proves: `bytes_scanned == total input len`); all `TEST_LOCK.lock().unwrap()` replaced with `.unwrap_or_else(|e| e.into_inner())` so a failure reports itself instead of cascading as `PoisonError` (same change in `tests/task_155.rs`).
- **Commit:** pending (wave 25 commit).
- **Why:** an integration test that invents the driver's contract proves nothing — the keys must come from the driver's `serde_json::json!` literals. The poison-tolerant lock keeps failures attributable: with it, the next real failure will show its own assertion instead of three `PoisonError`s.
- **Source:** the driver itself (`src/tasks/task_153.rs` metrics literals); `std::sync::Mutex` poisoning semantics.
- **Validation agents:** `cargo test -p phlow-gauntlet --test task_153` → 4/4 pass, including the allocator case (peak ≤ 64 KiB, no leak).
- **Adversarial agents:** the counting allocator is the adversarial instrument here — it measures the *driver's* `bench_parse` directly, so the allocation claim is evidence, not a byte-scan inference. Peak was asserted against `MAX_ENVELOPE_BYTES` with fixtures built outside the measured region.

## Full technical depth

`parse_envelope` trims the frame before parsing, so leading/trailing whitespace is accepted structurally, and JSON objects are unordered, so key order is irrelevant — both spellings decode to the same `serde_json::Value` and hence the same `Envelope`; the test asserts `got == want` on the typed value *and* equality of `to_canonical_json()`, so equivalence is proven at both levels. Cost is proven three ways: (1) `ParseStats.bytes_scanned` is set by the iterative depth pre-scan, which visits each byte once — the driver asserts `scanned == total input len` over 10,000 parses (exactly 1.0 per byte); (2) the `ManualClock` advances exactly one tick per parse, proving the loop is bounded and terminates (10,000 ticks, no more); (3) the integration test's counting global allocator (a `GlobalAlloc` wrapper around `System` tracking live bytes and peak) wraps `bench_parse` — the driver's exact entry point — and asserts peak live heap ≤ `MAX_ENVELOPE_BYTES` and no growth afterwards. The bound-size case parses one envelope at exactly 64 KiB, proving the bound admits legitimate traffic. The sweep (256→2048) shows `bytes_scanned == len` at every size: linear, no quadratic blowup hiding in the pre-scan.

## Sources

- Ghostex `de.rs` lenient-parsing rules @ c91146607205ac49303d1bcfe2fd6f9a86741500 (adaptation map).
- Tiger Style named bounds (`MAX_ENVELOPE_BYTES`).
- `crates/phlow-gauntlet/src/wire.rs` — `parse_envelope`, `ParseStats`.
- `crates/phlow-gauntlet/src/tasks/task_153.rs` (`bench_parse`), `crates/phlow-gauntlet/tests/task_153.rs` (counting allocator).
