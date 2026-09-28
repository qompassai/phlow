# task-164: half-open connection timeout

**Kind:** rust · **Status:** pass · **Wave:** 26 ·
**Commits:** single wave commit `gauntlet: wave 26 drivers, tests, docs`

## ELI5

Some attackers don't knock loudly — they open the door a crack and
then just stand there. A "slow-loris" peer connects to the daemon,
sends 3 bytes of a message header, and then goes silent forever,
hoping to hold a worker slot (and a socket) hostage. Do that 100
times and a naive daemon can't talk to anyone legitimate. This task
proves the daemon doesn't wait politely forever: every frame read
has a 250 ms deadline, so a stalled peer's slot is reaped, its file
descriptor closed, and no worker thread is left parked. After 100
simultaneous stallers are reaped, a legitimate client connects and
subscribes without a hiccup.

## What this task attempts

- **Goal:** stalled peers never hold an accept-path slot past the
  read deadline; fds return to baseline; legitimate clients are
  unaffected.
- **Mechanism:** `DaemonFixture` accept path with `FRAME_READ_TIMEOUT
  = 250 ms` per-frame read deadlines, in
  `crates/phlow-gauntlet/src/daemon_client.rs`; driven by
  `crates/phlow-gauntlet/src/tasks/task_164.rs` with real stalled
  TCP peers, a process fd census, and a thread census.
- **Success criterion:** 3-byte stall → slot released within
  `250 ms + 1 s` slack; 100 stallers → all reaped within
  `250 ms + 3 s`; fd count back to baseline; thread count back to
  baseline; a legitimate `subscribe` succeeds afterwards.
- **Non-goals:** rapid flapping (task 162), unauthenticated peers
  that complete their frames (task 163).

## What happened

Both adversarial cases pass on the final gates: `cargo test -p
phlow-gauntlet --test task_164` → 2 passed, 0 failed; lib 64/64; fmt
and clippy clean. A1: one peer sends 3 bytes then stalls → read
deadline fires at 250 ms, slot released (fixture `slots() == 0`),
fd count back to baseline, no parked worker thread, and the audit
log holds the read-timeout line. A2: 100 stalled peers at once →
all reaped within the `250 ms + 3 s` bound (`slots() == 0`), fds
`before == after`, and a legitimate client's `subscribe("legit")`
returns `subscribed` and is recorded by the daemon. The two tests in
the binary run serialized under a static mutex because the fd/thread
census is process-global.

## Where it went wrong

- **Stage:** A2 census measurement under the test harness.
- **Symptom:** fd/thread counts changed unpredictably between the two
  tests in the same binary — assertions saw phantom leaks.
- **Evidence:** two tests measuring process-global resources ran
  concurrently (the harness default); test A's live sockets were
  visible in test B's census window.
- **Root cause:** the census is process-wide, but the tests assumed
  per-test isolation. Fixed with a `static SERIAL: Mutex<()>` guard
  so the two tests in the binary never overlap (same guard added
  proactively to task 165, whose worker-thread census is likewise
  process-global).

- **Stage:** A2 helper extraction (≤70-line refactor).
- **Symptom:** clippy `-D unused-mut` on a leftover `let mut
  evidence` after conversion to the `finish` helper.
- **Root cause:** same leftover-`mut` class as task 163; removed.

## The fix — what changed and why

- **Changed:** `crates/phlow-gauntlet/tests/task_164.rs` — static
  `SERIAL` mutex serializing the two tests;
  `crates/phlow-gauntlet/src/tasks/task_164.rs` — `stall_peers`,
  `wait_for_reap`, `check_legitimate`, `finish` helpers; `let
  evidence` without `mut`.
- **Commit:** single wave commit `gauntlet: wave 26 drivers, tests,
  docs`.
- **Why:** serializing is the honest fix — the alternative (per-test
  fd namespaces) doesn't exist on Linux without containers, and
  subtracting a baseline per test would hide real leaks. The helpers
  keep each case ≤70 physical lines without changing what they
  assert.
- **Source:** Rust std `std::sync::Mutex` docs (poison-tolerant lock
  via `unwrap_or_else(|e| e.into_inner())`); slow-loris mitigation
  practice (bounded read deadlines on untrusted input).
- **Validation agents:** the subagent author ran the full gate set on
  primo after the fix: targeted 2/2, lib 64/64, fmt clean, clippy 0
  warnings — including repeated runs to shake out census flakiness.
- **Adversarial agents:** same author — tried 100 stallers *plus* one
  peer that sends its 3 bytes, waits 200 ms, sends 3 more (still
  under the 250 ms per-read deadline each time, but stalling the
  *frame*); the per-read deadline fires on the first silent 250 ms
  window and reaps it. A peer that dribbles 1 byte per 200 ms would
  hold a slot longer — documented as a known limitation; the bound
  is per-read, not per-frame.
- **New convention (if any):** none established — the serial-mutex
  pattern was already in use for process-global censuses; this just
  applies it.
- **Citations:** Rust std `TcpStream::set_read_timeout` semantics
  (deadline per `read` call); `/proc/self/fd` census via
  `daemon_client.rs:592`.

## Full technical depth

The accept path: `TcpListener::accept` → spawn (or reuse) a handler
→ `set_read_timeout(Some(FRAME_READ_TIMEOUT))` → read the 4-byte
length prefix → read the body. Any `read` that exceeds 250 ms returns
`WouldBlock`/timeout; the handler logs `read timeout`, closes the
socket, and releases the slot. Slots are counted by the fixture
(`slots()`) so the driver can assert reaping without timing the
daemon's internals.

A1's peer writes exactly 3 bytes of the 4-byte header then sleeps
indefinitely. The length-prefix read times out at 250 ms; the driver
allows 1 s of slack for CI scheduling jitter and asserts `slots() ==
0`, fd count restored, thread count restored, and the audit line
present. A2 scales to 100 concurrent stallers: the reap bound is
`250 ms + 3 s` (all slots share the same deadline regime, so reaping
is parallel, not serial — the +3 s is scheduling slack, not 100 ×
250 ms). Afterwards the driver drops its 100 held sockets, sleeps
100 ms for kernel fd release, and asserts the exact baseline fd
count; then a legitimate `subscribe` proves the accept path is
healthy.

Known limitation (documented, not fixed): the deadline is per `read`
call, so a peer dribbling ≥1 byte per 249 ms can hold a slot
indefinitely. A per-frame cumulative deadline would close this; it's
out of scope for this wave.

## Sources

- Primary: `crates/phlow-gauntlet/src/daemon_client.rs`
  (`DaemonFixture` accept path, `FRAME_READ_TIMEOUT = 250 ms`,
  `fd_count`, slot accounting).
- Primary: Rust std `std::net::TcpStream::set_read_timeout`,
  `std::time::{Duration, Instant}` docs.
- Secondary: wave 26 design notes
  (`~/workspace/gauntlet-design-tasks-151-200.md`, task-164 section);
  slow-loris mitigation practice (bounded read deadlines).
