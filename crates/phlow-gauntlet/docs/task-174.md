# task-174: single-use code consumption

**Kind:** rust · **Status:** pass · **Wave:** 28 · **Commits:** pending (wave 170-177)

## ELI5

A pairing code is a movie ticket: the usher tears it when you walk
in, and the torn half gets nobody a second seat. Here the "tear" is
a flag the daemon sets the instant a code pairs a phone — show the
same code again, even with the right secret, and the answer is
"already used". The tricky part is two people handing over the same
ticket at the *exact* same moment: the daemon handles the whole
check-and-tear as one uninterruptible step (one lock around lookup,
checks, compare, consume, and register), so exactly one of the two
gets in and the other is told the ticket is torn.

## What this task attempts

- **Goal:** a consumed code never pairs again, even under a
  concurrent double-presentation race.
- **Mechanism:** `crates/phlow-gauntlet/src/pairing.rs` —
  `Daemon::verify_inner` (the whole sequence under one `Mutex`;
  `consumed` flag set before device registration);
  driver `src/tasks/task_174.rs`, tests `tests/task_174.rs`.
- **Success criterion:** two adversarial cases pass — a replay of
  code + secret after a successful pairing is refused as `Consumed`
  with no second device and the replayed secret zeroized; two
  threads racing the same code (barrier-started) yield exactly one
  `PairingOk` and one `Consumed`, with exactly one device
  registered.
- **Non-goals:** expiry (task 171), forgery (task 175).

## What happened

Both cases pass on primo. The replay case presents, replays, and
asserts `Consumed`, an unchanged device count, and a zeroized replay
buffer. The race case shares one `Arc<Daemon>` between two threads
started from a `Barrier`, joins both, and asserts the 1/1 split and a
single registered device. Gates: `cargo build` clean, 2/2
integration tests, 64/64 lib tests, `cargo fmt --check` clean,
`cargo clippy --all-targets -D warnings` clean.

## Where it went wrong

- **Stage:** pre-gate review of the race case's thread-result
  handling.
- **Symptom:** the racer index was bound but unused in two match
  arms (`Ok((racer, Ok(_)))` ignored `racer`), which
  `cargo clippy -D warnings` rejects as `unused_variables`.
- **Evidence:** review of `src/tasks/task_174.rs`; no compiler
  output yet (gates pending).
- **Root cause:** leftover binding from a draft that reported per-
  racer outcomes; the final case only counts wins/losses.

## The fix — what changed and why

- **Changed:** `src/tasks/task_174.rs` — the two counting arms now
  bind `(_, ...)`; only the unexpected-outcome arm names the racer
  (it appears in the failure message).
- **Commit:** pending (wave 170-177)
- **Why:** the count is the assertion; naming an unused variable is
  noise the linter rightly refuses.
- **Source:** `rustc`/`clippy` `unused_variables` lint.
- **Validation agents:** primo gates — `cargo clippy --all-targets
  -D warnings` clean, task-174 2/2, full wave 16/16.
- **Adversarial agents:** the race case itself (barrier-started
  double presentation).
- **Citations:** none beyond the lint reference.

## Full technical depth

`verify_inner` runs lookup → consumed-check → expiry-check →
rate-limit → hash → constant-time compare → consume → register
without releasing the daemon mutex. Atomicity is what defeats the
race: whichever thread acquires the lock first completes the entire
sequence, including setting `consumed = true`, before the second
thread's lookup runs; the second thread then sees `consumed` and
gets `Consumed`. There is no check-then-act gap because there is no
gap between check and act — one critical section.

The device id is drawn *before* the code is consumed. If the entropy
draw fails (`PairingError::Entropy`, OS randomness unavailable), the
code stays live — the pairing never happened, so there is nothing to
roll back, and burning the code would punish the phone for a
daemon-side failure. Draw-before-consume keeps the flag's lifecycle
strictly monotonic; no rollback path exists.

Single-use consumption adapts the consumed-flag discipline from
Ghostex `server/src/remote_access/pairing_code.rs`; the
one-mutex atomicity argument and the barrier-started race proof are
phlow's own.

## Sources

- Ghostex `server/src/remote_access/pairing_code.rs` @
  `c91146607205ac49303d1bcfe2fd6f9a86741500` — consumed-flag
  discipline (primary).
- `crates/phlow-gauntlet/src/pairing.rs` — `verify_inner` (the
  single critical section), the entropy rollback comment.
