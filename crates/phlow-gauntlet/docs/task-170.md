# task-170: pairing code issuance

**Kind:** rust · **Status:** pass · **Wave:** 28 · **Commits:** pending (wave 170-177)

## ELI5

Your phone wants to pair with the phlow daemon on your computer. The
daemon makes a one-time secret code — like a temporary password that
only works once — and shows it to you. The code looks like
`phlow-ec1:` followed by scrambled text. Hidden inside that scrambled
text is a note saying when the code was made, how long it lives (15
minutes), and which phone it is for. The daemon does **not** keep a
copy of the secret itself; it keeps only a fingerprint (a hash) of it,
so even if someone reads the daemon's memory they cannot learn the
secret. When your phone shows the code back within 15 minutes, the
daemon checks the fingerprint, registers your phone, and throws the
code away — it can never be used again.

## What this task attempts

- **Goal:** the daemon mints well-formed one-time pairing codes and
  pairs a presenting phone exactly once.
- **Mechanism:** `crates/phlow-gauntlet/src/pairing.rs` —
  `Daemon::issue` (code minting, HMAC-SHA256 binding to the instance
  key), `Daemon::verify` (hash compare, atomic consume, device
  registration), `Daemon::store_dump` (plaintext-absence scan);
  driver `src/tasks/task_170.rs`, tests `tests/task_170.rs`.
- **Success criterion:** two validation cases pass — exact code
  shape with `issued_at`/`ttl_secs == 900`/label in the payload plus
  the store holding the secret's hash and never the secret; and one
  presentation pairing one device and consuming the code.
- **Non-goals:** expiry (task 171), timing side-channels (task 172),
  rate limiting (task 173), replay races (task 174), forgeries
  (task 175), sidecar supervision (task 176).

## What happened

Both cases pass on primo. The issuance case asserts the code starts
with `phlow-ec1:`, its payload JSON decodes with the scripted
`issued_at`, `ttl_secs` 900 and label `pixel-9`, the serialized store
contains `sha256(secret)` (positive control) and zero occurrences of
the secret bytes. The pairing case asserts one `verify` returns
`PairingOk` with one registered device and a second presentation
refused as `Consumed`. Gates: `cargo build` clean, 2/2 integration
tests, 64/64 lib tests, `cargo fmt --check` clean,
`cargo clippy --all-targets -D warnings` clean.

## Where it went wrong

Two iterations, both in the gate loop.

- **Stage:** first `cargo build` on primo.
- **Symptom:** 55 errors of the form `no method named 'now' found
  for struct ManualClock` (E0599) — one per driver.
- **Evidence:** `cargo build -p phlow-gauntlet` output; every task
  170–177 driver called `clock.now()`.
- **Root cause:** the drivers imported `ManualClock` but not the
  `Clock` trait that provides `now()`; Rust does not resolve trait
  methods without the trait in scope.

- **Stage:** first `cargo test` run (task 170).
- **Symptom:** `store_holds_hash_only` failed: "store dump lacks
  the secret hash (positive control failed)".
- **Evidence:** the dump rendered `sha256_hex(record.secret_hash)`
  while the driver searched for `sha256_hex(issued.secret)`.
- **Root cause:** double hashing — `record.secret_hash` is already
  SHA-256(secret), so hashing it again produced
  SHA-256(SHA-256(secret)) and the positive control could never
  match.

A third, design-level iteration: the wave's test budget is 16 total
(9 validation / 7 adversarial), and task 170 carried three cases.
The shape assertions and the hash-only store scan were merged into
one validation case (`issue_shape_and_hash_only`); `present_pairs_once`
stayed as the second.

## The fix — what changed and why

- **Changed:** all eight drivers — `use crate::bounty::clock::{Clock,
  ManualClock};`.
- **Commit:** pending (wave 170-177)
- **Why:** the trait must be in scope for method resolution. No
  alternative exists short of fully-qualified calls, which would be
  noise at every call site.
- **Source:** the Rust reference on trait method resolution
  (`Clock::now` is defined in `crates/phlow-gauntlet/src/bounty/clock.rs`).
- **Validation agents:** primo gate suite — `cargo build` 0 errors,
  all 16 integration tests pass.
- **Adversarial agents:** n/a for this iteration.

- **Changed:** `src/pairing.rs` — new `hex_encode` helper; the store
  dump renders the stored hash with `hex_encode(&record.secret_hash)`
  instead of `sha256_hex(...)`; `sha256_hex` is now defined as
  `hex_encode(&sha256_bytes(data))`.
- **Commit:** pending (wave 170-177)
- **Why:** the dump must render the *stored* hash (SHA-256(secret)),
  not hash it twice. The positive control is the proof the scan
  works — a scan that cannot find the hash cannot be trusted to
  report the secret's absence either.
- **Source:** `src/pairing.rs` `store_dump`; the driver's positive
  control in `src/tasks/task_170.rs`.
- **Validation agents:** primo gates — task-170 2/2, full wave
  16/16.
- **Adversarial agents:** the scan itself is the adversarial check
  (byte-window search for the secret across the whole dump).

- **Changed:** `src/tasks/task_170.rs`, `tests/task_170.rs` — merged
  `issue_shape` + `store_holds_hash_only` into
  `issue_shape_and_hash_only`; `CASES` is now 2 entries.
- **Commit:** pending (wave 170-177)
- **Why:** the wave budget is 16 tests (9V/7A); the merge keeps both
  assertions with one issue/verify round-trip and restores the
  count.
- **Source:** the wave-28 task brief (16 tests, 9 validation / 7
  adversarial).
- **Validation agents:** primo gates — task-170 2/2.
- **Adversarial agents:** n/a.

## Full technical depth

`Daemon::issue` takes a device label and a timestamp. It draws 16
random bytes for the code id and 32 for the secret from
`/dev/urandom` (failing closed with `PairingError::Entropy` if the OS
gives nothing), then binds the code to this daemon instance with
HMAC-SHA256 over `version|code_id|label|issued_at|ttl_secs` keyed by
a per-daemon 32-byte instance key. The payload JSON —
`{v, code_id, label, issued_at, ttl_secs, mac}` — is base64url-encoded
(no padding) and prefixed with `phlow-ec1:`. The store records the
code id, label, timestamps, `consumed: false`, an empty attempt
ledger, and **SHA-256(secret)** — the secret itself is returned to
the caller (the phone's credential) and never stored.

`Daemon::verify` parses and MAC-checks the code *before* any store
lookup, then under one mutex: looks the code up, rejects consumed and
expired codes, enforces the rate limit, hashes the presented secret
and compares digests in constant time, and on a match atomically
marks the code consumed and registers the device. The caller's secret
buffer is zeroized on every return path — including the error paths,
because `verify` zeroizes after `verify_inner` returns.

The `phlow-ec1:` prefix, the JSON shape, the per-instance HMAC
binding, and the loopback-only transport are phlow's own choices.
The 15-minute TTL, the one-time secret, and the hash-compare
discipline adapt Ghostex
`server/src/remote_access/pairing_code.rs` (re-implemented, never
ported).

## Sources

- Ghostex `server/src/remote_access/pairing_code.rs` @
  `c91146607205ac49303d1bcfe2fd6f9a86741500` — pairing-code concept,
  15-minute TTL, hash-compared single-use secret (primary).
- RFC 2104 — HMAC construction, re-implemented over `sha2`.
- RFC 4648 §5 — base64url alphabet, no padding.
- `crates/phlow-gauntlet/src/pairing.rs` — `issue` (~line 328),
  `verify_inner` (~line 448), `store_dump` (~line 595).
