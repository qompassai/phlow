# task-176: sidecar supervision under failure

**Kind:** rust · **Status:** pass · **Wave:** 28 · **Commits:** pending (wave 170-177)

## ELI5

The daemon can run a helper program — a "sidecar" — next to itself
for the remote pairing path (in production this would be the real
network helper; here it is a scripted stand-in, because the real
transport is out of scope). Helpers crash. The supervisor's job is
to notice, restart the helper a few times, and then give up
gracefully — "park" it — instead of restarting forever. Two things
must hold: a helper that dies instantly gets exactly three restarts
and then parks, and the daemon itself never goes down with it —
local pairing keeps working. And if the helper is murdered
*mid-conversation*, the caller gets a clean typed error
(`SidecarDown`) within seconds, never a hang.

## What this task attempts

- **Goal:** crash-looping sidecars are restarted exactly 3 times
  then parked; a SIGKILL mid-pairing surfaces as a typed error
  inside a 10 s bound; the daemon is unaffected either way.
- **Mechanism:** `crates/phlow-gauntlet/src/pairing.rs` —
  `SidecarSupervisor` (`start`/`poll`/`stop`/`kill_child`,
  `tunnel_roundtrip`), `Daemon::verify_remote` (loopback vs remote
  path); driver `src/tasks/task_176.rs`, tests `tests/task_176.rs`.
- **Success criterion:** two validation cases pass — the crasher
  parks as `Failed { restarts: 3 }`, further polls spawn nothing,
  and loopback pairing still works; the SIGKILL case's
  `verify_remote` returns `Err(SidecarDown)` within 10 s with the
  secret zeroized and no thread or child leaked.
- **Non-goals:** the real Tailscale transport (loopback is the
  stand-in), pairing logic itself (tasks 170–175).

## What happened

Both cases pass on primo. The crash-loop case drives a script that
exits 1 instantly: after exactly 3 restarts the supervisor parks in
`Failed { restarts: 3 }`, five more polls spawn nothing, and a
loopback issue+verify still pairs. The SIGKILL case runs a stalled
PONG script — it reads the PING then blocks on a second read that
never comes, spawning no subprocess — SIGKILLs it mid-round-trip
from another thread, and asserts `verify_remote` returns
`SidecarDown` promptly (observed well under the 10 s bound), with
no leaked child or pump thread. Gates: `cargo build` clean, 2/2 integration tests, 64/64 lib
tests, `cargo fmt --check` clean, `cargo clippy --all-targets -D
warnings` clean.

## Where it went wrong

Three real issues, two in `src/pairing.rs` found by the gate loop
(the first by the failing test, the second by review), the third a
fixture hazard found in review.

- **Stage:** `cargo test -p phlow-gauntlet --test task_176`.
- **Symptom:** `crash_loop_parks_bounded` failed: "supervisor did
  not park after 14 polls; status=Some(Stopped); restart count 1,
  want exactly 3".
- **Evidence:** the first `sidecar_poll` restarted the crasher
  (restarts=1) and every later poll returned `Stopped`.
- **Root cause:** `impl Drop for SidecarSupervisor` killed the child
  and set `Stopped` whenever *any* handle was dropped — including
  the temporary clones that `Daemon::sidecar_poll`,
  `sidecar_status`, and friends make per call (`self.lock().sidecar
  .clone()`). The first poll restarted the child, the temporary
  clone died at the end of the statement, and the `Drop` impl
  murdered the fresh child and stopped supervision.

- **Stage:** review of `poll()` while fixing the above.
- **Symptom (latent):** `poll()` reset `consecutive_failures` to 0
  whenever `try_wait` briefly observed a live child — including a
  child spawned milliseconds earlier that had not finished dying.
- **Evidence:** code review of the `alive` branch; the crash-loop
  case's own comment warned that a tight loop burns the poll budget
  on "still alive" observations.
- **Root cause:** one `try_wait` observation right after a spawn
  proves nothing (the child may be mid-exec), but it forgave the
  whole failure streak — a crash-looping sidecar could dodge the
  park threshold indefinitely under unlucky scheduling.

## The fix — what changed and why

- **Changed:** `src/tasks/task_176.rs` — the stalled-ponger
  fixture lost its `sleep 30`: it is now
  `while IFS= read -r _; do IFS= read -r _; echo PONG; done`.
- **Commit:** pending (wave 170-177)
- **Why:** `Child::kill` SIGKILLs only the direct child (the
  shell), not the process group. The `sleep 30` version orphaned
  the `sleep`, which held the stdout pipe open — the pump thread
  never saw EOF and the case passed only via the 2 s round-trip
  timeout, not via prompt EOF. The two-read blocker spawns no
  subprocess: it reads the PING, blocks on a second read that
  never comes, and dies with the shell, so stdout closes and EOF
  surfaces immediately. No orphaned processes, no 30 s linger.
- **Source:** reading `SidecarSupervisor::kill_child`
  (`Child::kill` semantics) against the fixture.
- **Validation agents:** primo `cargo test --test task_176` —
  the SIGKILL case's elapsed time should now be well under the
  2 s timeout instead of ~2 s.
- **Adversarial agents:** n/a for this iteration.

- **Changed:** `src/tasks/task_176.rs` — both case functions
  split at meaningful contracts (`drive_until_parked`,
  `check_parked_stable`, `prove_daemon_unaffected`;
  `prove_remote_alive`, `kill_mid_flight`,
  `prove_daemon_survived`) to satisfy the ≤70-physical-line rule.
- **Commit:** pending (wave 170-177)
- **Why:** the worktree AGENTS.md targets changed functions at
  ≤70 physical lines; the two cases were 82 and 114.
- **Source:** worktree `AGENTS.md` ("Tiger Style and
  performance") and the tiger-style-rust skill.
- **Validation agents:** primo full gate sweep after the split.
- **Adversarial agents:** n/a for this iteration.

- **Changed:** `src/pairing.rs` — removed `impl Drop for
  SidecarSupervisor`; cleanup moved to `impl Drop for
  SupervisorInner`, which kills and reaps the child only when the
  last `Arc` disappears (i.e. when the owning daemon is dropped).
- **Commit:** pending (wave 170-177)
- **Why:** temporary clones share the `Arc`; only the inner value's
  destruction means "nobody supervises this child anymore". The
  alternative (making `sidecar_poll` avoid the clone) would have
  hidden the hazard instead of removing it — any future clone would
  reintroduce the kill.
- **Source:** Rust `Arc`/`Drop` semantics; the failing assertion in
  `tests/task_176.rs`.
- **Validation agents:** primo gates — task-176 2/2 (parks after
  exactly 3 restarts), full wave 16/16.
- **Adversarial agents:** the crasher case itself; the SIGKILL case
  also exercises drop paths by killing mid-round-trip.

- **Changed:** `src/pairing.rs` — `poll()` no longer resets the
  streak on a single alive observation. A freshly (re)started child
  seen alive is promoted to `Running` without forgiveness; only a
  child that was *already* `Running` resets `consecutive_failures`.
- **Commit:** pending (wave 170-177)
- **Why:** "consecutive" should mean "not separated by a period of
  healthy operation", and one observation is not a period. Two
  consecutive alive observations now constitute health. The
  expected sequence is exact: four exits, three restarts, then
  `Failed { restarts: 3 }`.
- **Source:** the supervisor's own state machine in
  `src/pairing.rs`.
- **Validation agents:** primo gates — the crash-loop case asserts
  the exact restart count and parked status.
- **Adversarial agents:** n/a for this iteration.

## Full technical depth

`SidecarSupervisor` holds `Arc<Mutex<SupervisorInner>>` with the
child handles, a `consecutive_failures` streak, a `restarts`
counter, and a `SidecarStatus`. `start()` spawns once; `poll()` is
one supervision step: reap an exited child, bump the streak,
restart while `streak <= MAX_SIDECAR_RESTARTS` (3), else park in
`Failed`. `stop()` is explicit and idempotent. The pump thread
forwards the child's stdout lines over an mpsc channel and exits on
EOF, so a dead child always releases its thread — no thread
outlives its process.

`tunnel_roundtrip` is the remote path's health check: write PING,
wait for PONG with a 2 s bound (`TUNNEL_ROUNDTRIP_TIMEOUT`). Any
failure — dead child, broken pipe, wrong reply, timeout — is
`SidecarDown`. `ChildStdin` has no `try_clone`, so the handle is
taken out of the supervisor while the lock is released and put back
afterwards; a supervision change in between just drops the handle,
and the next round-trip re-checks liveness first.

`verify_remote` picks the path: without a sidecar (or with a dead
one) it runs the loopback verify; with a live sidecar it does the
tunnel round-trip first. The SIGKILL case forces the remote path
with a stalled PONG script (reads the PING, then blocks on a second
read that never comes — no subprocess, so the SIGKILLed shell's
stdout closes and the pump thread's EOF surfaces promptly),
SIGKILLs the child mid-round-trip from a second thread, and
requires `Err(SidecarDown)` inside 10 s — the 2 s round-trip
timeout plus scheduling slack. The secret buffer
is zeroized on this path too, and the case asserts no child or
thread survives.

The restart/park discipline adapts Ghostex
`server/src/tailcat/supervisor.rs`; the `Arc`-scoped `Drop`, the
two-observation health rule, the take/put-back stdin handling, and
the loopback/remote split are phlow's own.

## Sources

- Ghostex `server/src/tailcat/supervisor.rs` and
  `server/src/tailcat/types.rs` @
  `c91146607205ac49303d1bcfe2fd6f9a86741500` — sidecar supervision
  concept (primary).
- `crates/phlow-gauntlet/src/pairing.rs` — `SidecarSupervisor`
  (~line 690), `tunnel_roundtrip` (~line 830), `verify_remote`
  (~line 533).
