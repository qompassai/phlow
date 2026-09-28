# task-159: reconnect ladder

**Kind:** rust · **Status:** pass · **Wave:** 26 ·
**Commits:** single wave commit `gauntlet: wave 26 drivers, tests, docs`

## ELI5

Imagine your phone call keeps dropping. A naive phone would redial
instantly, over and over — burning battery and hammering the network.
A polite phone waits a little longer each time: 100 ms, then 500 ms,
then 2 seconds, and so on. That waiting schedule is the *reconnect
ladder*. This task proves our daemon client climbs that ladder: after
three dropped connections it waits the right amounts, the fourth
attempt succeeds, and the ladder resets to the bottom because success
earns a fresh start. It also proves the client gives up gracefully —
after 10 straight failures it *parks* (stops trying on its own) instead
of spinning forever.

## What this task attempts

- **Goal:** reconnect attempts follow the `[100, 500, 2000, 4000, 8000,
  16000]` ms ladder, reset on success, and park after
  `MAX_RECONNECT_ATTEMPTS` with no busy loop.
- **Mechanism:** `ReconnectEngine` in
  `crates/phlow-gauntlet/src/daemon_client.rs` driven by
  `crates/phlow-gauntlet/src/tasks/task_159.rs` against a
  `ScriptedLink` double and a `ManualClock`
  (`crates/phlow-gauntlet/src/bounty/clock.rs`).
- **Success criterion:** attempt timestamps match the ladder's
  cumulative sums within one 1 s tick; after the cap the engine reports
  `ParkReason::BackoffExhausted` and schedules zero further wakeups.
- **Non-goals:** subscription continuity (task 160), state convergence
  (task 161), adversarial flapping (task 162).

## What happened

Both driver cases pass on the final gates: `cargo test -p
phlow-gauntlet --test task_159` → 2 passed, 0 failed; lib tests 64/64;
`cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings`
clean. V1 shows attempt timestamps at exactly the ladder's cumulative
offsets from `T0_MS = 1_000_000_000` with deviation 0, then a healthy
30 s stream plus an orderly close resets `consecutive_failures` to 0
and the next delay to the 100 ms floor. V2 drives the daemon down for
the whole run: exactly 10 attempts, then parked with
`BackoffExhausted`, and zero further engine wakeups scheduled.

## Where it went wrong

Two bugs surfaced during development, both caught by the driver's own
assertions before any gate ran.

- **Stage:** V1 timestamp verification.
- **Symptom:** attempt timestamps read `2_000_000_100`,
  `2_000_000_600`, … — the epoch added twice.
- **Evidence:** driver evidence line `attempt timestamps (ms):
  [2000000100, 2000000600, 2000002600, 2000026600]` vs want
  `[1000000100, 1000000600, 1000002600, 1000026600]`.
- **Root cause:** the driver treated the engine's absolute-ms deadlines
  as offsets and re-based them onto `T0_MS`. The engine already speaks
  one absolute timeline; only the *first* attempt is anchored at T0.

- **Stage:** both cases' verdict plumbing.
- **Symptom:** `report.passed == false` while `report.failures` was
  empty — a case could never honestly report.
- **Evidence:** `CaseReport::pass(...)` builds a passing report, but the
  driver never copied its local `failures` vec into it before
  recomputing `passed`.
- **Root cause:** missing assignment `report.failures = failures`
  before `report.passed = report.failures.is_empty()`.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_159.rs` —
  `drive_drops` anchors only the first flap at `T0_MS`; every deadline
  the engine returns via `next_attempt_at_ms()` is consumed verbatim.
  Every driver case in the wave assigns `report.failures = failures`
  before computing `passed` (here via the shared `finish` helper).
- **Commit:** single wave commit `gauntlet: wave 26 drivers, tests,
  docs`.
- **Why:** the engine's contract is "one absolute-ms timeline"; the
  driver must not re-base it. The alternative — making the engine
  return offsets — would push clock arithmetic into every consumer;
  keeping the engine absolute keeps the driver honest and the engine
  simple.
- **Source:** Ghostex `packages/gx-client/src/worker.rs` reconnect
  loop @ c911466 (ladder + reset-on-healthy-close doctrine);
  `RECONNECT_LADDER_MS` values are ours (Ghostex uses
  `[250, 1000, 2000, 4000, 8000, 16000]`; wave 26 deliberately uses a
  faster floor of 100 ms).
- **Validation agents:** the subagent author ran the full gate set on
  primo after the fix: targeted 2/2, lib 64/64, fmt clean, clippy 0
  warnings.
- **Adversarial agents:** same author, red-teaming the fix — verified
  V2 parks with zero scheduled wakeups (no busy loop by construction:
  a parked engine has no next deadline) and that a healthy-but-abrupt
  close does *not* reset the counter (only healthy + orderly does).
- **Citations:** `worker.rs` ladder/reset doctrine; Rust std
  `std::time` docs for the wall-clock-free `ManualClock` design
  (`crates/phlow-gauntlet/src/bounty/clock.rs`).

## Full technical depth

`ReconnectEngine` holds `consecutive_failures: u32`,
`parked: Option<ParkReason>`, and `attempt_times_ms: Vec<u64>`.
`note_attempt(now_ms)` returns false once parked. `note_disconnect(at,
orderly)` increments the counter and schedules the next attempt at
`at + RECONNECT_LADDER_MS[min(failures-1, 5)]` — unless the disconnect
was healthy (stream age ≥ `HEALTHY_STREAM_MS = 30_000`) *and* orderly,
in which case the counter resets to 0. At `MAX_RECONNECT_ATTEMPTS =
10` consecutive failures the engine parks with
`ParkReason::BackoffExhausted` and `next_attempt_at_ms()` returns
`None` forever: no timer, no thread, no wakeup — parking is structural,
not a sleep loop.

The driver (`drive_drops`, `check_reset`, `drive_until_parked`) runs on
a `ManualClock` starting at `T0_MS / 1000` seconds. The `ScriptedLink`
is scripted `connect → drop` three times, then accepts; reads are
scripted per phase. V1 asserts each attempt timestamp equals
`T0_MS + cumulative_ladder[i]` within one 1 s tick (the tick absorbs
the ms↔s clock granularity), then asserts the reset. V2 asserts the
attempt count is exactly 10, the park reason, and that
`next_attempt_at_ms()` is `None` (zero future wakeups — the
busy-loop check). Every changed function is ≤70 physical lines;
`#![forbid(unsafe_code)]` holds crate-wide.

## Sources

- Primary: Ghostex `packages/gx-client/src/worker.rs`,
  `packages/gx-client/src/config.rs` @ c91146607205ac49303d1bcfe2fd6f9a86741500
  (reconnect loop, ladder, reset-on-success doctrine).
- Primary: `crates/phlow-gauntlet/src/daemon_client.rs:55`
  (`RECONNECT_LADDER_MS`), `:59` (`MAX_RECONNECT_ATTEMPTS`), `:63`
  (`HEALTHY_STREAM_MS`), `:116-128` (`ParkReason`, engine state).
- Primary: `crates/phlow-gauntlet/src/bounty/clock.rs`
  (`ManualClock` — deterministic time).
- Secondary: wave 26 design notes
  (`~/workspace/gauntlet-design-tasks-151-200.md`, task-159 section)
  for the scenario/criterion wording.
