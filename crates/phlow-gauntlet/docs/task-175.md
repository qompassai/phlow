# task-175: forged code rejection

**Kind:** rust · **Status:** pass · **Wave:** 28 · **Commits:** pending (wave 170-177)

## ELI5

Someone intercepts a real pairing code and starts flipping bits in
it, hoping a mangled copy still opens the door. The daemon's answer
has layers: gibberish that is not even shaped like a code is thrown
out immediately; a well-shaped code whose tamper-seal (a keyed MAC)
does not check out is thrown out next; and neither kind ever gets
as far as the secret check — the attacker learns nothing about the
secret from 200 tries. A code made by a *different* daemon (a
different secret key) is equally worthless here, even though it is
perfectly well-formed. And after all that abuse, the daemon's own
real code still works fine — the forgeries left no trace.

## What this task attempts

- **Goal:** every forged or foreign code is refused before the
  secret comparison, with zero pairings and an unpoisoned store.
- **Mechanism:** `crates/phlow-gauntlet/src/pairing.rs` —
  `Daemon::parse_code` (prefix, base64url, JSON, MAC ordering),
  `Daemon::verify_inner` (gates before compare),
  `Daemon::hash_comparisons` (instrumentation); driver
  `src/tasks/task_175.rs`, tests `tests/task_175.rs`.
- **Success criterion:** two adversarial cases pass — 200
  bit-flipped forgeries all die as `Malformed` or `Authenticity`
  with the comparison counter unmoved and zero devices; a foreign
  instance's code and a `ghostex-ec1:`-prefixed code die as
  `Authenticity`/`Malformed` with no compare, and the home daemon's
  genuine code still pairs afterwards.
- **Non-goals:** timing (task 172), rate limiting (task 173),
  expiry (task 171).

## What happened

Both adversarial cases pass on primo. The fuzz case flips one bit
per copy across 200 deterministic (PCG-seeded) positions of a
genuine code: every forgery is refused, none reaches the
constant-time compare, none pairs. The foreign-instance case
verifies the MAC gate binds codes to the minting daemon — same
shape, wrong key, refused as `Authenticity` — and that the
`ghostex-ec1:` prefix is refused as `Malformed` even with a genuine
body. Gates: `cargo build` clean, 2/2 integration tests, 64/64 lib
tests, `cargo fmt --check` clean, `cargo clippy --all-targets -D
warnings` clean.

## Where it went wrong

One gate-loop iteration, in the driver (not the daemon).

- **Stage:** `cargo test -p phlow-gauntlet --test task_175`.
- **Symptom:** `foreign_instance_and_prefix` failed:
  `comparisons_unchanged` was false.
- **Evidence:** the driver captured `comparisons_before`, ran the
  three forgeries, then verified the *genuine* code (which
  legitimately performs one compare), and only then compared the
  counter.
- **Root cause:** the counter was read after the genuine verify —
  the case measured its own proof-of-health as a forgery leak.

## The fix — what changed and why

- **Changed:** `src/tasks/task_175.rs` — the case captures
  `comparisons_after_forgeries` immediately after the three forgery
  attempts, before the genuine verify; the metric and assertion use
  that value.
- **Commit:** pending (wave 170-177)
- **Why:** the property under test is "forgeries never reach the
  compare"; the genuine verify is a separate property ("the store
  is unpoisoned") and legitimately compares. Reading the counter
  between them keeps the two properties from contaminating each
  other.
- **Source:** `src/pairing.rs` `Daemon::hash_comparisons` and the
  `verify_inner` gate ordering.
- **Validation agents:** primo gates — task-175 2/2, full wave
  16/16.
- **Adversarial agents:** the cases themselves are the red team
  (200-forgery corpus, foreign key, wrong prefix, truncated
  garbage).

A compile-time iteration from the same gate loop: clippy's
`unusual_hex` lint rejected the PCG seed literal `0x175_F0_26`
(hex digits not in equal groups); it is now `0x0175_F026`. No
behavior change — the seed only needs to be deterministic.

## Full technical depth

`parse_code` enforces the refusal order before any store access:
prefix check → base64url decode → JSON parse → required-field
validation → HMAC-SHA256 verification against the daemon's instance
key. Each layer maps to a typed error: bad prefix/shape is
`Malformed`, a well-formed body with a bad MAC is `Authenticity`.
Only a code that survives all five reaches the store lookup in
`verify_inner`, and the secret compare sits behind three more gates
(consumed, expired, rate-limited).

The bit-flip corpus is deterministic: a PCG generator seeded with
`0x0175_F026` picks 200 (byte, bit) positions in a genuine code and
flips one bit per copy. Corruption in the prefix or base64url body
tends to produce `Malformed`; corruption inside the payload or MAC
tends to produce `Authenticity`. The case asserts the union covers
all 200 and that `hash_comparisons` does not move — the observable
proof that no forgery reached the compare.

Instance binding is the HMAC key: `Daemon::new` draws a fresh
32-byte key from `/dev/urandom`, so two daemons never share one
(except in fixtures). A code minted by daemon B presented to daemon
A has a valid shape and a valid MAC — under the wrong key — and
dies at the MAC gate. The replay half of task 177's ceremony case
reuses this property to prove transcripts are instance-bound.

The layered refusal order and the MAC-before-lookup discipline
adapt Ghostex `server/src/remote_access/pairing_code.rs`'s
validation sequence; the PCG corpus, the wrong-prefix probe, and
the compare-counter instrumentation are phlow's own.

## Sources

- Ghostex `server/src/remote_access/pairing_code.rs` @
  `c91146607205ac49303d1bcfe2fd6f9a86741500` — code validation
  sequence (primary).
- `crates/phlow-gauntlet/src/pairing.rs` — `parse_code`,
  `verify_inner`, `Daemon::hash_comparisons`.
