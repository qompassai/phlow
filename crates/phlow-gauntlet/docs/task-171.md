# task-171: pairing code TTL expiry

**Kind:** rust · **Status:** pass · **Wave:** 28 · **Commits:** pending (wave 170-177)

## ELI5

A pairing code is like a parking ticket that expires. The daemon
stamps every code with the exact minute it was made, and the rule is
simple: 15 minutes later, to the second, the code is dead. Show up at
14 minutes 59 seconds and you get in; show up at exactly 15 minutes
and the door stays shut. Dead codes also do not pile up in a drawer
forever — a cleanup job throws away every expired code, so the store
never grows without bound.

## What this task attempts

- **Goal:** codes presented at or after `issued_at + 900 s` are
  refused as `Expired`, and expired records are purged in bounded
  time.
- **Mechanism:** `crates/phlow-gauntlet/src/pairing.rs` —
  `Daemon::verify_inner` (the `now.saturating_sub(issued_at) >=
  TTL_SECS` boundary), `Daemon::purge_expired`; driver
  `src/tasks/task_171.rs`, tests `tests/task_171.rs`.
- **Success criterion:** two validation cases pass — T+899 s
  accepted, T+900 s and T+901 s refused as `Expired`; 1,000 expired
  codes purged with 3 live codes surviving, inside a 5 s bound.
- **Non-goals:** issuance shape (task 170), replay/consumption
  (task 174), rate limiting (task 173).

## What happened

Both cases pass on primo. The boundary case issues a fresh code per
probe (a successful verify consumes its code) and checks three
points: T+899 s → `PairingOk`, T+900 s → `Expired`, T+901 s →
`Expired`. The purge case plants 1,000 expired codes and 3 live
codes, purges exactly the expired set with the 3 live codes
surviving, issues 3 fresh codes after the purge and pairs with them
(post-purge service intact), all inside a 5 s bound. Gates: `cargo
build` clean, 2/2 integration tests, 64/64 lib tests, `cargo fmt
--check` clean, `cargo clippy --all-targets -D warnings` clean.

## Where it went wrong

One gate-loop iteration.

- **Stage:** `cargo test -p phlow-gauntlet --test task_171`.
- **Symptom:** `purge_expired_bounded` failed: `assertion left ==
  right failed, left: 6, right: 3` on the `remaining` metric.
- **Evidence:** the driver read `daemon.code_count()` for the
  `remaining` metric *after* issuing 3 survivor-probe codes, so the
  count was 3 live + 3 probes = 6.
- **Root cause:** metric captured at the wrong point in the case's
  timeline — after the post-purge probes instead of right after the
  purge.

## The fix — what changed and why

- **Changed:** `src/tasks/task_171.rs` — the case captures
  `remaining_after_purge = daemon.code_count()` immediately after
  `purge_expired`, before any survivor-probe codes are issued; the
  assertion and the metric use that value.
- **Commit:** pending (wave 170-177)
- **Why:** the purge's contract is about the store state the purge
  leaves behind; anything issued afterwards is a different
  experiment. The alternative (subtracting the probe count) would
  couple the metric to the probe setup instead of measuring the
  purge directly.
- **Source:** `src/tasks/task_171.rs` `case_purge_expired_bounded`.
- **Validation agents:** primo gates — task-171 2/2, full wave
  16/16.
- **Adversarial agents:** n/a for this iteration.

## Full technical depth

The expiry check lives in `verify_inner`, ordered before the
rate-limit and compare steps: `now.saturating_sub(record.issued_at)
>= TTL_SECS` → `Err(PairingError::Expired)`. The `saturating_sub`
makes clock skew (a `now` earlier than `issued_at`) fail safe toward
"not expired" rather than underflowing. The boundary is inclusive at
the TTL second: elapsed 899 accepts, elapsed 900 refuses — the
spec's "boundary exact at TTL second".

`purge_expired(now)` retains records with `expires_at >= now` and
returns the removed count. It is a single linear pass over the code
map — O(n) with a tiny constant — so 1,000 records purge in well
under a millisecond; the 5-second bound in the driver is generous on
purpose (it guards against a future regression to something
pathological, not against today's cost). The purge keeps live codes
fully usable: the case issues fresh codes after the purge and pairs
with them.

The 15-minute TTL adapts Ghostex `REMOTE_PAIRING_SECRET_TTL`; the
exact-second boundary semantics and the purge cadence are phlow's
own.

## Sources

- Ghostex `server/src/remote_access/pairing_code.rs` @
  `c91146607205ac49303d1bcfe2fd6f9a86741500` — pairing-code TTL
  concept (primary).
- `crates/phlow-gauntlet/src/pairing.rs` — `verify_inner` expiry
  check, `purge_expired`.
