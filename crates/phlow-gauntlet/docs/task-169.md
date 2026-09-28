# task-169: effect ordering and idempotency

**Kind:** rust (adversarial) · **Status:** pass (after 2 fixes) · **Wave:** 27 · **Commit:** pending (wave 27)

## ELI5

The cook (task 167) is in the kitchen; now the *delivery driver* bringing prep lists from the head office is under test. Three things can go wrong on the road. First, the driver might deliver the same list twice — so every list carries a stamp (an idempotency key), and the second delivery of a stamped list is thrown away with a polite "already got it." Second, some lists have no stamp because they can't be safely repeated (a notification you can't un-send) — if the driver tries to re-deliver one of those, the kitchen must refuse loudly rather than send it twice. Third, lists might arrive out of order — the kitchen parks early arrivals on a small numbered shelf (a bounded hold buffer) and applies them once the missing ones show up; if the shelf overflows, it rejects the excess instead of growing forever.

## What this task attempts

- **Goal:** verify the transport path's exactly-once-ish contracts: keyed duplicates are typed no-ops, keyless retries are refused, out-of-order deliveries park in a bounded buffer and are rejected past the bound.
- **Mechanism:** `crates/phlow-gauntlet/src/state_machine.rs` — `Delivery{seq, key, effect}`, `MockInterpreter::deliver` (idempotency ledger, `next_seq`, bounded `hold` buffer of `HOLD_BUFFER_MAX=16`), `EffectError::{Duplicate, RetryRefused, OutOfOrder, Stale}`. Driver: `crates/phlow-gauntlet/src/tasks/task_169.rs` (`gap_fill_check`, `bound_overflow_check`); integration tests in `crates/phlow-gauntlet/tests/task_169.rs`.
- **Success criterion:** A1 — same keyed delivery twice → `Duplicate`, ledger shows exactly 1 application. A2 — keyless effect re-presented → `RetryRefused`, applied once. A3 — seqs [0,2,1] → ledger [0,1,2]; 16 future deliveries held, the 17th → `OutOfOrder{seq:18, expected:1}`; delivering seq 1 then drains the buffer in order.
- **Non-goals:** batch atomicity (task 167); reducer/ingestion behavior (tasks 166/168); real network transport (the mock is the declared backend).

## What happened

PASS, all gates green on primo:

- **A1:** keyed persist delivered twice → second returns `EffectError::Duplicate{key: "k-1"}`; ledger holds 2 entries (persist 1, persist 2) with `persist_task_1_applications == 1` — zero double-applied side effects.
- **A2:** keyless notify delivered, then re-presented as a retry → `EffectError::RetryRefused{seq: 0}`; `notify_applications == 1`.
- **A3:** `deliver(0)` applies; `deliver(2..=17)` parks (16 held = `HOLD_BUFFER_MAX`); `deliver(18)` → `OutOfOrder{seq: 18, expected: 1}`; `deliver(1)` fills the gap and drains the buffer — final ledger is the gapless sequence 0..=17 in order. 3/3 tests pass; full gate suite green.

## The fix — what changed and why

- **Changed (fix 1):** the shared `report.failures = failures;` fix (see task-167 doc) — preventive here; 169's cases already passed.
- **Changed (fix 2):** `crates/phlow-gauntlet/src/tasks/task_169.rs` — `Err(EffectError::RetryRefused { seq }) if seq == 0` → `Err(EffectError::RetryRefused { seq: 0 })` (clippy `redundant_guards` under `-D warnings`).
- **Changed (fix 3):** split `case_out_of_order_bounded_hold` into `gap_fill_check` + `bound_overflow_check` (70-line Tiger Style ceiling).
- **Commit:** pending (wave 27).
- **Why:** fix 2 is pure lint hygiene — the pattern `seq: 0` says the same thing as the guard with no runtime cost and no clippy complaint. Fix 3 follows event/variant responsibility like the other splits.
- **Source:** clippy's `redundant_guards` lint documentation for fix 2; the crate's `AGENTS.md` 70-line rule for fix 3.
- **Validation agents:** worker ran the full gate suite on primo: 10/10 integration, 64/64 lib, fmt clean, clippy 0 warnings.
- **Adversarial agents:** the three cases are the red team. Known limitation, stated openly: duplicate detection scans the applied ledger only, so a duplicate of a delivery still parked in the hold buffer is not detected as a duplicate (it would be held again, then applied once when the gap fills — still exactly-once in effect, but the typed `Duplicate` signal fires only post-application). This matches the declared contract and is pinned by the current tests; widening detection into the hold buffer is a deliberate future option, not a bug.
- **Citations:** `crates/phlow-gauntlet/src/state_machine.rs` (`MockInterpreter::deliver`, hold-buffer logic, `EffectError` variants).

## Full technical depth

`deliver` runs a decision tree on every delivery, in this order:

1. **Keyed duplicate?** If `key` is `Some` and the key already appears in the applied ledger → `Err(Duplicate{key})`, no state change. The ledger is the side-effect record, so "already applied" is defined as "in the ledger" — this is what makes the no-op *typed* rather than silent.
2. **Keyless retry?** If `key` is `None` and the (seq, effect-kind) was already applied → `Err(RetryRefused{seq})`. Keyless effects (like notifications) declare themselves non-idempotent; the interpreter would rather fail loud than double-apply. The "already applied" check for keyless deliveries keys on seq, since there is no idempotency key to consult.
3. **Stale?** If `seq < next_seq` → `Err(Stale{seq, expected: next_seq})` — a late duplicate of an old sequence, distinct from a keyed duplicate.
4. **Future?** If `seq > next_seq` → park in `hold` if `hold.len() < HOLD_BUFFER_MAX`, else `Err(OutOfOrder{seq, expected: next_seq})`. The bound is what keeps a malicious or buggy sender from growing memory without limit; 16 is a named constant, not a magic number.
5. **Apply** (`seq == next_seq`): append to fs/notify logs and the cross-sink sequence, record the ledger entry, bump `next_seq`, then drain: while the head of `hold` has `seq == next_seq`, apply it too. The drain loop is what turns [0,2,1] into ledger [0,1,2].

The hold buffer is a `Vec<Delivery>` scanned linearly — fine at bound 16, and the bound is exactly what keeps the scan cheap. A `BinaryHeap` would be the speculative-optimization version of this; the task deliberately doesn't.

Atomicity note: `deliver` applies a single delivery, so there is no batch-atomicity question here — that contract lives one layer up in `interpret_batch` (task 167). The two compose: the batch layer rejects unknown kinds before anything applies; the transport layer guarantees each accepted delivery applies exactly once, in seq order.

## Sources

- `~/workspace/scratch/ghostex/packages/gx-core/src/core.rs` — effect/interpreter split motivating the transport contracts (primary source, Ghostex @ c91146607205ac49303d1bcfe2fd6f9a86741500, MIT)
- `~/workspace/ghostex-recon/adaptation-map.md` — adapted pattern vs. non-lifted domain
- `crates/phlow-gauntlet/src/state_machine.rs` — `Delivery`, `LedgerEntry`, `MockInterpreter::deliver`, `EffectError::{Duplicate, RetryRefused, OutOfOrder, Stale}`, `HOLD_BUFFER_MAX`
- `crates/phlow-gauntlet/src/tasks/task_169.rs`, `crates/phlow-gauntlet/tests/task_169.rs`
- `~/workspace/gauntlet-design-tasks-151-200.md` — task-169 design (Wave 27)
