# task-168: hostile event injection

**Kind:** rust (adversarial) · **Status:** pass (after 1 fix) · **Wave:** 27 · **Commit:** pending (wave 27)

## ELI5

Before the recipe book (task 166) ever sees an order ticket, a bouncer checks it at the door. The bouncer turns away tickets for dishes that don't exist (unknown event kinds), tickets with nonsense table numbers (task ids 0 and 18-quintillion-minus-1, which are reserved), and tickets where someone wrote a 10-megabyte novel in the "dish name" box. The key trick: the bouncer *measures* the novel without *photocopying* it — refusing on length alone, so the attack costs the kitchen nothing. And the bouncer never panics, no matter how weird the ticket looks.

## What this task attempts

- **Goal:** verify the ingestion boundary refuses unknown variants and hostile payloads with typed errors, zero panics, unchanged state, and no allocation spike.
- **Mechanism:** `crates/phlow-gauntlet/src/state_machine.rs` — `ingest(&RawEvent) -> Result<Event, EventError>` with named bounds (`MAX_LABEL_BYTES=256`, `MAX_REASON_BYTES=1024`, `INGEST_ALLOC_BUDGET_BYTES=65536`); `EventError::{Unknown, OversizedPayload, OutOfRangeIndex, BadPercent}`. Driver: `crates/phlow-gauntlet/src/tasks/task_168.rs` (`hostile_ingestion_batch` under `catch_unwind`); integration tests in `crates/phlow-gauntlet/tests/task_168.rs`, including a test-local counting global allocator around `ingest`.
- **Success criterion:** A1 — 6 unknown variants → `EventError::Unknown`, state byte-identical. A2 — 10 MB label → `OversizedPayload{label, 10485760, 256}`; ids 0/`u64::MAX` → `OutOfRangeIndex`; percent 101 → `BadPercent`; oversized reason → `OversizedPayload{reason}`; zero panics; refusing the 10 MB label allocates ≤ 64 KB.
- **Non-goals:** reducer transition legality (task 166 — ingestion only admits well-formed events; the reducer still enforces its own invariants); transport behavior (task 169).

## What happened

PASS, all gates green on primo:

- **A1:** 6 unknown variants (`teleport`, `""`, `SPAWN`, …) → `EventError::Unknown` each; state byte-identical before/after.
- **A2:** 10 MB label → `OversizedPayload{field: "label", bytes: 10485760, max: 256}`; `u64::MAX` and `0` → `OutOfRangeIndex`; percent 101 → `BadPercent`; 1025-byte reason → `OversizedPayload{field: "reason"}`; a benign `start` still ingests — bounds are enforced, not weaponized against legitimate traffic. Whole batch under `catch_unwind`: 0 panics. State byte-identical.
- **Allocation:** the integration test's counting global allocator measured the 10 MB refusal: allocation delta across `ingest` ≤ 64 KB budget (the refusal path allocates nothing — length check precedes any clone). 3/3 tests pass; full gate suite green (64/64 lib, fmt, clippy 0 warnings).

## The fix — what changed and why

Two iterations, both about honesty of measurement:

- **Changed (iteration 0, pre-gate):** an early draft of the driver asserted the allocation bound with a placeholder probe that did not actually measure allocation. It was deleted before any gate ran and replaced with the real counting allocator in the integration test. It never passed a gate and is documented here so the history is honest: the first instinct (assert without measuring) was wrong, and the fix was to measure.
- **Changed (iteration 1):** the shared `report.failures = failures;` fix (see task-167 doc) — preventive here; 168's cases already passed.
- **Commit:** pending (wave 27).
- **Why:** a bound that isn't measured is a wish. The counting allocator (`GlobalAlloc` delegating to `System`, test-binary only — the library crate keeps `#![forbid(unsafe_code)]`) makes the "no allocation spike" claim falsifiable: if `ingest` ever cloned the hostile string before checking its length, the delta would read ~10 MB against a 64 KB budget and fail loudly. The 160× margin (64 KB vs 10 MB) keeps the test robust against cross-thread allocator noise from parallel tests.
- **Source:** Rust std docs for `std::alloc::GlobalAlloc` (the measurement mechanism); the crate's `#![forbid(unsafe_code)]` posture (why the `unsafe impl` lives in the test binary, not the library).
- **Validation agents:** worker ran the full gate suite on primo: 10/10 integration, 64/64 lib, fmt clean, clippy 0 warnings.
- **Adversarial agents:** the A1/A2 cases are themselves the red team (unknown variants, 10 MB string, sentinel ids, bad percent, oversized reason, plus a benign control). `catch_unwind` converts any panic anywhere in the batch into a case failure.
- **Citations:** `std::alloc::{GlobalAlloc, System}` (rust std); `crates/phlow-gauntlet/src/state_machine.rs` (`ingest`, `check_text_len`, `check_task_id`, named bound constants).

## Full technical depth

The ingestion boundary exists because the reducer's types are too trusting: `Event::Spawn{label: String}` will happily hold 10 MB. `ingest` is the single choke point where untrusted wire data (`RawEvent`, with its plain `String` fields) becomes typed events, and the ordering inside it is load-bearing:

1. **Id first** (`check_task_id`): 0 and `u64::MAX` are refused before the kind is even matched — a hostile id never reaches variant-specific logic.
2. **Kind second**: unknown kinds → `EventError::Unknown{kind}` (the kind string is echoed for diagnosability, bounded by the fact that it came from the wire and is only logged, never executed).
3. **Payload bounds third, per variant**: `check_text_len` compares `text.len()` against the named bound and returns *before* any `.clone()`. This is what makes the allocation claim structural rather than incidental — the 10 MB string is never copied, so the refusal's allocation delta is ~0 by construction, and the counting-allocator test pins that construction.

`BadPercent` (101) is checked before the `as u8` cast — the cast itself can't panic for 0..=255 inputs, but the check keeps the invariant ("percent ≤ 100") at the boundary instead of relying on reducer re-validation. Defense in depth: `reduce` still rejects `BadProgress`, but ingestion is where the typed, user-facing refusal lives.

Zero-panic is enforced two ways: by construction (no indexing by untrusted values anywhere on the path — `find_task` is a linear scan returning `Option`) and by test (`catch_unwind` around the whole hostile batch; a panic becomes a failure, not a crash).

The named bounds are the Tiger Style contract made numeric: `MAX_LABEL_BYTES`, `MAX_REASON_BYTES`, `MAX_TASKS`, `MAX_EFFECTS_PER_EVENT`, `INGEST_ALLOC_BUDGET_BYTES` — every limit has a name, a single definition site, and a test that exercises it.

## Sources

- `~/workspace/scratch/ghostex/packages/gx-core/src/core.rs` — events-in boundary as the trust boundary (primary source, Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500, MIT)
- `crates/phlow-gauntlet/src/state_machine.rs` — `ingest`, `ingest_spawn/progress/block/fail`, `check_task_id`, `check_text_len`, `EventError`, named bounds
- `crates/phlow-gauntlet/src/tasks/task_168.rs`, `crates/phlow-gauntlet/tests/task_168.rs` (counting allocator)
- `~/workspace/gauntlet-design-tasks-151-200.md` — task-168 design (Wave 27)
