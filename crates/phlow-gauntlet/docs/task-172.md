# task-172: constant-time secret compare

**Kind:** rust · **Status:** pass · **Wave:** 28 · **Commits:** pending (wave 170-177)

## ELI5

Imagine checking whether two long numbers match by reading them
digit by digit and stopping at the first difference. A spy with a
stopwatch can tell *where* you stopped — and that leaks how many
leading digits of the secret they guessed right. The fix is to always
read all 32 digits, whether they match or not, so the stopwatch
learns nothing. This task also makes sure the secret itself is wiped
from memory the moment it is used (overwritten with zeros), and that
no copy of the secret hides in the daemon's stored records or its
logbook.

## What this task attempts

- **Goal:** secret comparison leaks no timing signal, and the
  plaintext secret exists nowhere after use.
- **Mechanism:** `crates/phlow-gauntlet/src/pairing.rs` —
  `ct_compare` (no-early-exit 32-byte compare with a visit counter),
  `zeroize`, `Daemon::verify`/`verify_remote` (zeroize on every
  return path), `Daemon::store_dump`, `Daemon::audit_log`; driver
  `src/tasks/task_172.rs`, tests `tests/task_172.rs`.
- **Success criterion:** two validation cases pass — measured
  accept/reject loop times stay within a 2.0x factor, `ct_compare`
  visits exactly 32 bytes on match and on mismatch, caller buffers
  are all-zero after verify on both paths, and the store dump plus
  audit log contain the hash but never the secret.
- **Non-goals:** rate limiting (task 173), forgery rejection
  (task 175).

## What happened

Both cases pass on primo. The timing case pre-issues 2,000 fresh
codes per arm (correct secret vs. a 32-byte wrong secret), verifies
one code per iteration, sanity-checks that the arms actually
accepted/mismatched, and asserts the per-iteration time ratio stays
under 2.0. The zeroization case verifies the accept and mismatch
paths leave all-zero caller buffers and scans the store dump and
audit log for the secret bytes, with the hash as the positive
control. Gates: `cargo build` clean, 2/2 integration tests, 64/64 lib
tests, `cargo fmt --check` clean, `cargo clippy --all-targets -D
warnings` clean.

## Where it went wrong

One design iteration, caught in the gate loop.

- **Stage:** `cargo test -p phlow-gauntlet --test task_172`.
- **Symptom:** `timing_side_channel_bounded` failed: "sanity arms
  wrong: (Err(Consumed), Err(RateLimited)), want (Ok, Mismatch)".
- **Evidence:** the first draft ran all 2,000 iterations of each arm
  against a single code. The accept arm's first verify consumed its
  code (the other 1,999 measured the `Consumed` path); the reject
  arm's first five verifies were `Mismatch` and the rest measured
  the `RateLimited` path. The sanity probes then inherited those
  terminal states.
- **Root cause:** the timing loops measured the wrong code paths —
  a correct verify consumes its code and repeated wrong secrets
  trip the rate limiter, so reusing one code per arm cannot measure
  the compare.

## The fix — what changed and why

- **Changed:** `src/tasks/task_172.rs` — the case pre-issues one
  fresh code per iteration per arm (4,000 codes), the timing helper
  takes an index, and the sanity probes use their own fresh codes.
- **Commit:** pending (wave 170-177)
- **Why:** each iteration must exercise the full accept or mismatch
  path — parse, MAC, gates, hash, constant-time compare — for the
  ratio to mean anything. Issue cost is identical in both arms, so
  it cancels out of the factor. The alternative (resetting the
  consumed flag / clearing the attempt ledger between iterations)
  would have measured a synthetic path no real caller ever takes.
- **Source:** `src/pairing.rs` `verify_inner` — consume-on-accept
  and the sliding-window limiter, the two mechanisms that made
  code-reuse unmeasurable.
- **Validation agents:** primo gates — task-172 2/2 (factor under
  the 2.0 bound on the first post-fix run), full wave 16/16.
- **Adversarial agents:** n/a for this iteration.

## Full technical depth

`verify_inner` never compares the presented secret directly. It
hashes the presented bytes with SHA-256, then compares the two
32-byte digests with `ct_compare`: a loop over all 32 positions
accumulating `diff |= a[i] ^ b[i]` with no early exit, returning
`(diff == 0, visited)`. The `visited` counter is the testable
contract — the driver asserts 32 on both the match and the mismatch
path, so the property is proved, not assumed.

Timing discipline: accept and reject do identical work (hash, then
the full 32-byte walk). The measured factor bound of 2.0 is generous
for a correct constant-time routine (expect ~1.0) and tight enough
to catch an early-exit byte loop, which would show several-x on a
32-byte digest when the first byte differs. The loop count (2,000
iterations per arm) averages out scheduler noise; the sanity arms
guard against measuring two no-op paths.

Zeroization: `verify` and `verify_remote` call `zeroize(secret)`
after `verify_inner`/`verify_remote_inner` returns, on *every* path
— including `Malformed`, `Authenticity`, `Unknown`, `Expired`,
`Consumed`, `RateLimited`, `Mismatch`, and `SidecarDown`. The helper
writes through `&mut [u8]`, so the stores are observable through the
borrow. The store dump and audit log are scanned byte-wise for the
secret; the hash's presence proves the scan would have found a leak.

The visit-count instrumentation, the timing bound, and the
zeroization contract are phlow's own; the hash-compared secret check
adapts Ghostex `server/src/remote_access/pairing_code.rs`.

## Sources

- Ghostex `server/src/remote_access/pairing_code.rs` @
  `c91146607205ac49303d1bcfe2fd6f9a86741500` — hash-compared secret
  check (primary).
- The constant-time-compare discipline follows the standard
  accumulate-differences construction (no data-dependent branches on
  secret material).
- `crates/phlow-gauntlet/src/pairing.rs` — `ct_compare`, `zeroize`,
  `verify`.
