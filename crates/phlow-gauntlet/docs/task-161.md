# task-161: snapshot resync after reconnect

**Kind:** rust · **Status:** pass · **Wave:** 26 ·
**Commits:** single wave commit `gauntlet: wave 26 drivers, tests, docs`

## ELI5

While your call was down, five things happened that you missed. When
you redial, you don't want to hear them out of order, twice, or not at
all. So the other side first sends you a *snapshot* — "here is
everything as of message #5" — and only then resumes the live stream.
The tricky part: live messages can arrive *while* the snapshot is still
in flight. This task proves the client handles that race: it holds the
early live messages aside, applies the snapshot first, throws away any
live message the snapshot already covered, then applies the rest in
order. A forged or outdated snapshot is rejected outright.

## What this task attempts

- **Goal:** after an outage the client converges to exactly the
  daemon's state via snapshot + live stream, with zero gaps and zero
  duplicates — even when live events race the snapshot.
- **Mechanism:** `ClientState` (`Snapshot { version, as_of, state }`
  application + live-event buffer) in
  `crates/phlow-gauntlet/src/daemon_client.rs`, driven by
  `crates/phlow-gauntlet/src/tasks/task_161.rs` against scripted
  daemon state and a `ManualClock`.
- **Success criterion:** deep-equal client/daemon state after resync
  in both scenarios; the event ledger shows zero gaps and zero
  duplicates; a stale snapshot is rejected with state untouched.
- **Non-goals:** subscription replay (task 160), hostile snapshots
  from unauthenticated peers (task 163 covers the auth layer; the
  staleness check here is the client's own).

## What happened

Both driver cases pass on the final gates: `cargo test -p
phlow-gauntlet --test task_161` → 2 passed, 0 failed; lib 64/64; fmt
and clippy clean. V1: 5 missed events → snapshot `as_of=5` applied →
live resumes → `applied_through = 8`, client state deep-equals daemon
state, ledger shows 0 dup-dropped and 0 gaps. V2: snapshot `as_of=5`
captured *before* racing live events 6 and 7 arrive → both buffered,
snapshot applied first, buffered events at/below `as_of` dropped as
duplicates, events 6–8 applied in order → deep-equal, and a stale
`{version: 4, as_of: 90}` replay is rejected with `StaleSnapshot` and
the client state untouched.

## Where it went wrong

- **Stage:** V2 race setup.
- **Symptom:** racing live events 6 and 7 were duplicate-dropped —
  the driver reported them as already covered, so the final state was
  missing them.
- **Evidence:** snapshot arrived with `as_of=7`; the buffer held
  events 6 and 7; both were dropped as ≤ `as_of`.
- **Root cause:** the driver captured the snapshot *after* creating
  events 6 and 7. The snapshot must be captured at sequence 5 *before*
  the racing events are produced — otherwise `as_of` is a lie about
  what the snapshot covers.

- **Stage:** V2 ledger assertion.
- **Symptom:** expected the ledger to contain only snapshot/applied
  records; it didn't match.
- **Evidence:** ledger `[(5, "snapshot"), (6, "buffered"), (7,
  "buffered"), (8, "applied"), …]` plus the replay disposition.
- **Root cause:** the `ClientState` ledger is append-only and records
  *every* disposition honestly — buffered, applied, dup-dropped, and
  the stale-replay rejection. The test's expectation was wrong, not
  the ledger. Fixed the assertion to check the full honest ledger.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/src/tasks/task_161.rs` —
  `race_and_drain` captures the snapshot at sequence 5 before
  producing racing events 6 and 7; assertions check the complete
  ledger including `buffered` entries and the replay disposition.
- **Commit:** single wave commit `gauntlet: wave 26 drivers, tests,
  docs`.
- **Why:** `as_of` is a contract — "this snapshot covers everything
  through sequence N". Capturing it late makes the contract false and
  the buffer logic correctly drops the "already covered" events. The
  alternative (redefining `as_of` loosely) would corrupt the dedup
  invariant the whole design rests on.
- **Source:** the `as_of` marker contract is ours (wave 26 design,
  task-161 section); the snapshot-then-buffer ordering follows
  Ghostex's snapshot-resync doctrine (`packages/gx-client` @ c911466).
- **Validation agents:** the subagent author ran the full gate set on
  primo after the fix: targeted 2/2, lib 64/64, fmt clean, clippy 0
  warnings.
- **Adversarial agents:** same author — replayed a stale snapshot
  mid-race (rejected, state untouched), and delivered live event 8
  twice (second copy dup-dropped, applied exactly once).
- **Citations:** Ghostex snapshot-resync; Rust std `Vec`/`HashMap`
  docs for the ledger and state map (no external protocol spec — the
  frame contract is defined in `daemon_client.rs`).

## Full technical depth

`ClientState` holds `version: u64`, `applied_through: u64`,
`state: HashMap<String, String>`, and an append-only
`ledger: Vec<(u64, &'static str)>`. The resync protocol:

1. On reconnect the client requests a snapshot.
2. While awaiting it, live events are *buffered*, not applied.
3. The snapshot applies first: `state` is replaced wholesale,
   `version`/`as_of` recorded, ledger gets `(as_of, "snapshot")`.
4. Buffered events with `seq <= as_of` are duplicate-dropped (the
   snapshot already covers them); `seq > as_of` apply in sequence
   order; any `seq` gap is a typed error, never silently skipped.
5. A snapshot with `version < current_version`, or `version ==
   current_version` but `as_of < current_as_of`, is rejected as
   `StaleSnapshot` with state untouched.

Backoff resets only after a healthy *and* orderly close — an abrupt
drop never resets, even one synchronized exactly to the
`HEALTHY_STREAM_MS` boundary (task 162 weaponizes that boundary).
Named bounds: `MAX_FRAME_BYTES = 64 KiB` caps any single frame the
snapshot can arrive in.

## Sources

- Primary: Ghostex `packages/gx-client/src/worker.rs` @
  c91146607205ac49303d1bcfe2fd6f9a86741500 (snapshot-resync
  doctrine; the `as_of` marker contract is ours).
- Primary: `crates/phlow-gauntlet/src/daemon_client.rs`
  (`ClientState`, `Snapshot`, ledger).
- Secondary: wave 26 design notes
  (`~/workspace/gauntlet-design-tasks-151-200.md`, task-161 section).
