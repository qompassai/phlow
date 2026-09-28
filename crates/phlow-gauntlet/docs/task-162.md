# task-162: flapping daemon backoff

**Kind:** rust · **Status:** pass · **Wave:** 26 ·
**Commits:** single wave commit `gauntlet: wave 26 drivers, tests, docs`

## ELI5

Task 159 proved the client redials politely when the line drops by
accident. But what if the *other side is attacking you* — picking up
and hanging up a thousand times to turn your polite redialer into a
hammer? This task is the attack version. It proves two things: first,
that 1,000 rapid accept-then-drop flaps still cost the client only 10
attempts (the cap), spaced by the real ladder, with no leaked sockets
or file descriptors. Second — the sneaky one — that flaps timed to
land *exactly* when the ladder would reset don't trick the client
into collapsing back to rapid-fire redials. The failure counter
survives the ambush.

## What this task attempts

- **Goal:** under adversarial flapping the client stays bounded —
  attempts capped, intervals never below the ladder floor, no
  thread/fd leaks — and boundary-synced drops never collapse the
  backoff.
- **Mechanism:** `ReconnectEngine` + `ScriptedLink` (accept/drop
  scripting) in `crates/phlow-gauntlet/src/daemon_client.rs`, driven
  by `crates/phlow-gauntlet/src/tasks/task_162.rs` on a `ManualClock`
  with a process fd census (`fd_count`).
- **Success criterion:** ≤ `MAX_RECONNECT_ATTEMPTS` attempts over
  1,000 flaps; first-to-last attempt span ≥ the ladder sum (78,600
  ms); minimum inter-attempt interval ≥ 100 ms floor; fd count and
  link handle census return to baseline; after 7 boundary-synced
  drops the next delay is the 16,000 ms top rung, not the floor.
- **Non-goals:** benign drops (task 159), slow-loris stalls (task
  164).

## What happened

Both adversarial cases pass on the final gates: `cargo test -p
phlow-gauntlet --test task_162` → 2 passed, 0 failed; lib 64/64; fmt
and clippy clean. A1: 1,000 accept-then-drop flaps → exactly 10 client
attempts, parked `BackoffExhausted`; attempt span 78,600 ms =
`100+500+2000+4000+8000+16000*4`; minimum inter-attempt interval 100
ms (the floor); link census `connects=10 closes=10 open=0`; fd count
unchanged. A2: 7 drops synced exactly to the healthy-stream boundary
→ `consecutive_failures = 7` (the counter survived every reset
point); post-flap delay 16,000 ms (top rung, not the 100 ms floor);
then one genuine healthy + orderly close → counter resets to 0 and
the next delay is the floor, proving the reset path still works when
earned.

## Where it went wrong

- **Stage:** A1 census helper extraction (refactor for the ≤70-line
  bound).
- **Symptom:** `cargo build` failed: `error[E0308]: mismatched types:
  expected usize, found u64` and `can't compare usize with u64` in
  `check_census`.
- **Evidence:** `fd_count()` returns `usize`; the extracted helper
  declared its parameter as `u64`.
- **Root cause:** the refactor changed the parameter type while moving
  code that previously compared two `fd_count()` results inline.
  Fixed the signature to `usize` — no behavior change.

No behavioral bug was found in the engine's flap handling: the
boundary-synced ambush failed against the counter on the first run,
which is the point of the task.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_162.rs` —
  `check_census(&ScriptedLink, fds_before: usize, …)`; the six
  `finish` helpers across the wave take `case: &'static str` to match
  `CaseReport::pass`'s contract (task 162's was one of them).
- **Commit:** single wave commit `gauntlet: wave 26 drivers, tests,
  docs`.
- **Why:** `fd_count` counts entries in `/proc/self/fd` — a `usize`
  quantity. Matching the helper to the source type keeps the
  comparison honest; casting would hide a future type change.
- **Source:** Rust std `usize` semantics; the `CaseReport::pass`
  signature in `crates/phlow-gauntlet/src/skillopt/driver.rs`.
- **Validation agents:** the subagent author ran the full gate set on
  primo after the fix: targeted 2/2, lib 64/64, fmt clean, clippy 0
  warnings.
- **Adversarial agents:** same author — the A2 case itself is the
  red-team: 7 drops landing exactly on the `HEALTHY_STREAM_MS`
  boundary, the precise moment a naive "stream lived long enough,
  reset the counter" implementation would collapse. The ladder stayed
  escalated.
- **Citations:** Tiger Style bounded-retries principle (no unbounded
  loops, own-and-release-exactly-once for the 10 accepted sockets);
  Ghostex ladder @ c911466.

## Full technical depth

The attack model: the daemon (or the network path) accepts the TCP
connection and immediately drops it — cheap for the attacker, and
each flap is a chance for a naive client to reconnect instantly.
`ReconnectEngine` defeats it structurally: the delay after failure *n*
is `RECONNECT_LADDER_MS[min(n-1, 5)]`, so delays escalate
100→500→2000→4000→8000→16000 and *stay* at 16000; at 10 consecutive
failures the engine parks with no further deadline. A parked engine
performs zero wakeups by construction — there is no sleep loop to
become a busy loop.

The subtle attack (A2) targets the *reset* rule: backoff resets only
after a healthy (≥30 s) *and* orderly close. An abrupt drop — even one
synchronized exactly to the 30,000 ms boundary — increments the
counter instead. The driver's 7 synced flaps prove the counter reaches
7 and the scheduled delay is the top rung (16,000 ms); a naive reset
would have scheduled the 100 ms floor after every flap. The positive
control then shows a genuine healthy + orderly close *does* reset to
0 with the next delay at the floor — the reset path isn't dead, it's
just not free.

Census: `ScriptedLink` counts connects/closes/open handles (every
accepted socket released exactly once); `fd_count()` snapshots
`/proc/self/fd` before and after the storm.

## Sources

- Primary: Ghostex `packages/gx-client/src/worker.rs` @
  c91146607205ac49303d1bcfe2fd6f9a86741500 (ladder doctrine).
- Primary: `crates/phlow-gauntlet/src/daemon_client.rs:55`
  (`RECONNECT_LADDER_MS`), `:59` (`MAX_RECONNECT_ATTEMPTS`), `:63`
  (`HEALTHY_STREAM_MS`), `:592` (`fd_count`).
- Secondary: wave 26 design notes
  (`~/workspace/gauntlet-design-tasks-151-200.md`, task-162 section).
