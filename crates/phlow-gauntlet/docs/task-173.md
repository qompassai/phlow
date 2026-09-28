# task-173: brute-force rate limiting

**Kind:** rust · **Status:** pass · **Wave:** 28 · **Commits:** pending (wave 170-177)

## ELI5

Someone steals your pairing code display — or just keeps guessing
secrets against your code, thousands of tries. The daemon keeps a
tally per code: five wrong guesses within one minute, and the sixth
guess is turned away *without even checking the secret*. The attacker
learns nothing from the blocked tries, the code still works for its
real owner once the minute passes, and every blocked try is written
in the logbook with the code's label. Trying the same trick against
a *different* code starts a fresh tally — codes do not punish each
other.

## What this task attempts

- **Goal:** a wrong-secret hammering attack is throttled per code
  without ever reaching the secret comparison, and sustained abuse
  pairs nothing.
- **Mechanism:** `crates/phlow-gauntlet/src/pairing.rs` —
  `verify_inner` (per-code attempt ledger, `RateLimited` before the
  hash compare), `Daemon::hash_comparisons` (instrumentation),
  `Daemon::audit_log`; driver `src/tasks/task_173.rs`, tests
  `tests/task_173.rs`.
- **Success criterion:** two adversarial cases pass — five
  mismatches in 40 s, sixth attempt `RateLimited` with the
  comparison counter unmoved, correct secret also `RateLimited`
  inside the window, code usable after the window; then a 100-attempt
  storm with zero pairings, first `RateLimited` at attempt six, one
  labeled audit entry per violation, and a sibling code with a fresh
  budget.
- **Non-goals:** expiry (task 171), forgery (task 175).

## What happened

Both adversarial cases pass on primo. Case A1: five mismatches in
40 s, sixth attempt `RateLimited` with the comparison counter
unmoved, correct secret also `RateLimited` inside the window, code
usable after the window. Case A2: a 100-attempt storm at one
scripted instant yields zero pairings, first `RateLimited` at
attempt six, one labeled audit entry per violation (95), and a
sibling code under the same label pairing fine on its own fresh
budget. Gates: `cargo build` clean, 2/2 integration tests, 64/64
lib tests, `cargo fmt --check` clean, `cargo clippy --all-targets
-D warnings` clean.

## Where it went wrong

- **Stage:** pre-gate review of the storm case's scripted timeline.
- **Symptom:** the first draft advanced the scripted clock 1 s per
  storm attempt, so the 60 s sliding window expired mid-storm:
  attempts 65–70 would have been admitted as fresh `Mismatch`es and
  the expected violation count (95) would not have materialized —
  the case would have failed its own audit-count assertion.
- **Evidence:** desk-check of the `retain` predicate
  (`now - t < RATE_LIMIT_WINDOW_SECS`) against the attempt schedule
  t = 1..=100: attempts at t = 1..=5 fall out of the window by
  t = 65, reopening the budget.
- **Root cause:** the storm was scripted as a trickle across 100 s
  while the limiter's window is 60 s; a real hammering attack is a
  burst, and the test should be one too.

## The fix — what changed and why

- **Changed:** `src/tasks/task_173.rs` — the storm case no longer
  advances the clock between attempts; all 100 land at the same
  scripted instant (removed the now-unused `STORM_STEP_SECS`).
- **Commit:** pending (wave 170-177)
- **Why:** with every attempt inside one window, attempts 1–5 are
  `Mismatch` and 6–100 are `RateLimited` — exactly the 95 audit
  entries the case asserts. The alternative (keeping the trickle and
  asserting the messier 89-violation pattern) would have tested the
  test's arithmetic instead of the limiter. The time-spread behavior
  is still covered: case A1 advances the clock and proves the code
  recovers after the window.
- **Source:** the limiter's own `retain` predicate in
  `src/pairing.rs` (`verify_inner`); the sliding-window discipline
  adapts Ghostex `server/src/tailcat/supervisor.rs`'s
  `RateLimiter`.
- **Validation agents:** primo gates — task-173 2/2, full wave
  16/16.
- **Adversarial agents:** the A2 storm case itself (100-attempt
  burst, sibling-budget isolation); the pre-gate review also played
  adversary by desk-checking the window-slide scenario — the exact
  trick a real attacker uses to stretch a budget — and the limiter
  handles it by construction (old attempts age out, the budget
  reopens: throttling, not bricking).

A second, compile-time iteration came from the gate loop:

- **Changed:** `src/tasks/task_173.rs` — the storm case binds
  `daemon.audit_log()` to a local `audit` before filtering it into
  `violations: Vec<&String>`.
- **Commit:** pending (wave 170-177)
- **Why:** `audit_log()` returns an owned `Vec`; borrowing the
  temporary across the `collect()` is E0716 (temporary dropped
  while borrowed). Binding it first extends the lifetime to the
  enclosing block — the standard fix.
- **Source:** `rustc` E0716.
- **Validation agents:** primo `cargo build` clean after the fix.
- **Adversarial agents:** n/a for this iteration.
- **Citations:** `src/pairing.rs` `verify_inner` rate-limit block.

## Full technical depth

Each code record carries an `attempts: Vec<u64>` ledger. On every
verify, `verify_inner` first prunes entries with
`now - t >= RATE_LIMIT_WINDOW_SECS` (and any `t > now`, so clock
jumps cannot poison the ledger), then refuses with `RateLimited` if
5 or more remain — *before* hashing or comparing the presented
secret. The refusal is logged with the code's label. Only then is
the attempt recorded and the compare run.

The ordering is the security property: `RateLimited` precedes the
compare, so a throttled attacker burns no comparison oracle. The
`hash_comparisons` counter makes this observable — case A1 asserts
it does not move across the blocked attempt. The window slides
rather than locks: after 60 s the budget reopens and the legitimate
code works again (asserted in A1), and per-code scoping means a storm
against one code never affects its siblings (asserted in A2 with a
second code under the same label).

The per-code window and the check-before-compare ordering are
phlow's own; the sliding-window discipline adapts Ghostex's
`RateLimiter`.

## Sources

- Ghostex `server/src/tailcat/supervisor.rs` @
  `c91146607205ac49303d1bcfe2fd6f9a86741500` — `RateLimiter`
  discipline (primary).
- `crates/phlow-gauntlet/src/pairing.rs` — `verify_inner`
  rate-limit block, `Daemon::hash_comparisons`.
