# task-167: effect interpreter separation

**Kind:** rust (validation) · **Status:** pass (after 2 fixes) · **Wave:** 27 · **Commit:** pending (wave 27)

## ELI5

Task 166 proved the recipe book only *writes down* prep lists. Task 167 checks the cook who *executes* them. The cook must do the prep list in exactly the written order — write, notify, write — even though writes and notifications go to different sinks (a filesystem log and a notification log). And if the book ever contains a dish the cook doesn't know (an unknown effect kind), the cook must refuse the *entire* list without cooking a single item first — not cook two dishes and then complain about the third.

## What this task attempts

- **Goal:** verify the interpreter executes recorded effects in exact cross-sink order, and rejects a batch containing an unknown effect kind atomically with a typed error.
- **Mechanism:** `crates/phlow-gauntlet/src/state_machine.rs` — `MockInterpreter` (in-memory fs/notify doubles, cross-sink `sequence` log, `interpret_batch` with validate-before-apply atomicity); `Effect::VendorExtension` as the unknown kind. Driver: `crates/phlow-gauntlet/src/tasks/task_167.rs`; integration tests in `crates/phlow-gauntlet/tests/task_167.rs`.
- **Success criterion:** V1 — the 6-effect script `[persist 1 ×3, notify 1, persist 2 ×2]` applies in exactly that cross-sink order. V2 — a batch containing `VendorExtension{"future-effect"}` returns `EffectError::Unhandled`, with zero writes and zero notifications applied.
- **Non-goals:** reducer purity (task 166); transport ordering/idempotency (task 169); real filesystem or notification sinks (the mocks are the declared test backend).

## What happened

First attempt FAILED at the test gate:

- **V1** reported `passed=false` with an *empty* failure message: `task-167 case effects_execute_in_recorded_order failed: ` (nothing after the colon).

## Where it went wrong

Two stacked defects, found in order:

- **Stage:** `cargo test -p phlow-gauntlet --test task_167`, case `effects_execute_in_recorded_order`.
- **Symptom:** the case failed, but `report.failures` was empty — the verdict said "failed" with no diagnosis.
- **Evidence:** an instrumented run printed:
  ```
  DBG effects.len=6
  DBG failures.len=1 is_empty=false
  DBG report.passed=false report.failures=[]
  ```
  The local `failures` vec held the real finding (`reducer emitted 6 effects, want 7`) while the report's `failures` field was empty.
- **Root cause (1):** the driver's count assertion expected 7 effects; the script emits 6 (spawn→1, start→1, complete→2, spawn→1, start→1). Plain arithmetic miscount in the fixture expectation.
- **Root cause (2):** every wave-27 driver built the report via `CaseReport::pass(...)` then set `report.passed = failures.is_empty()` — but never assigned `report.failures = failures`. The diagnosis lived only in the local vec (folded into `evidence` as strings) while the report's typed `failures` field stayed empty. Root cause 1's symptom was therefore undebuggable from the report alone; only `eprintln!` instrumentation revealed it.

## The fix — what changed and why

- **Changed (fix 1):** `crates/phlow-gauntlet/src/tasks/task_167.rs` — expected effect count 7 → 6 (assertion, evidence string, and metrics); extracted `recorded_order_script()` so the script and its assertions are separate functions.
- **Changed (fix 2):** `task_166.rs`, `task_167.rs`, `task_168.rs`, `task_169.rs` — added `report.failures = failures;` after every `report.passed = failures.is_empty();` (7 sites).
- **Changed (fix 3, Tiger Style):** split `reduce` into per-event `reduce_*` functions behind a shared `transition` guard, split `ingest` into per-variant `ingest_*` helpers plus `check_task_id`/`check_text_len`, and split the over-long case functions (`replay_once`/`replay_report`, `hostile_ingestion_batch`, `gap_fill_check`/`bound_overflow_check`) — all functions now ≤70 physical lines.
- **Commit:** pending (wave 27).
- **Why:** fix 1 addresses the miscount directly — the script is transition-legal by construction, so the expectation was wrong, not the reducer. Fix 2 restores the `CaseReport` contract ("`failures`: Failing assertion details, empty when `passed`"): a failing report must carry its diagnosis, otherwise every future failure repeats this debugging session. Fix 3 is the workspace's standing 70-line rule; the splits follow event/variant responsibility, no speculative abstraction.
- **Source:** `crates/phlow-gauntlet/src/skillopt/driver.rs` (`CaseReport` field docs) for fix 2; the crate's own `AGENTS.md` ("Target changed functions at most 70 physical lines") for fix 3.
- **Validation agents:** worker re-ran the full gate suite on primo after all fixes: `cargo build` clean; `cargo test` 10/10 integration (2+2+3+3) and 64/64 lib; `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` 0 warnings.
- **Adversarial agents:** none beyond the gate suite; the V2 unknown-effect case itself is the adversarial half (atomic batch reject with zero side effects observed on both mocks).
- **New convention (if any):** none established — but a lesson recorded: when a case fails with an empty diagnosis, suspect the reporting path before the code under test.

## Full technical depth

The interpreter contract under test is "effects in, side effects out, in order." `MockInterpreter::interpret_batch` implements the atomicity half as *validate-before-apply*: it scans the whole batch for unknown kinds first and only then applies anything. The alternative — apply-then-rollback — was rejected because the mocks have no rollback mechanism and inventing one would test the mock, not the contract. With validate-first, `EffectError::Unhandled{name}` fires with the fs and notify logs provably empty, which V2 asserts as `fs_writes_after_reject == 0` and `notifies_after_reject == 0`.

V1's order assertion is on the cross-sink `sequence` log (`"fs:persist task 1"`, `"notify:notify task 1: task 1 done"`, …), not on per-sink counts. Per-sink counts would pass if the notify ran before the writes; only the interleaved sequence proves the interpreter honored emission order across sinks. The `[Write, Notify, Write]` core the design brief names is entries 2..=4 of the 6-entry sequence (persist 1 from complete, notify 1, persist 2 from spawn) — the full 6-entry assertion is strictly stronger.

`Effect::VendorExtension{name}` is the declared extension point for future real sinks; the interpreter treats any unknown variant as `Unhandled` rather than dropping it silently — a silent drop would turn a misconfigured deployment into lost persistence with no signal.

## Sources

- `~/workspace/scratch/ghostex/packages/gx-core/src/core.rs` — interpreter separation: the core returns effects, execution lives elsewhere (primary source, Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500, MIT)
- `~/workspace/ghostex-recon/adaptation-map.md` — adapted pattern vs. non-lifted domain
- `crates/phlow-gauntlet/src/state_machine.rs` — `MockInterpreter::interpret_batch`, `Effect::VendorExtension`, `EffectError::Unhandled`
- `crates/phlow-gauntlet/src/tasks/task_167.rs`, `crates/phlow-gauntlet/tests/task_167.rs`
- `~/workspace/gauntlet-design-tasks-151-200.md` — task-167 design (Wave 27)
